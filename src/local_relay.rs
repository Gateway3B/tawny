//! A relay on the phone itself, between the player and YouTube.
//!
//! Streams resolved on the phone (`local_streams`) are fetched straight from
//! YouTube, but the player cannot ask for them directly: Shaka fetches media
//! with `fetch`, and YouTube's media servers send no CORS headers, so the
//! WebView refuses the responses. They also want the request headers yt-dlp
//! resolved them with. This relay listens on 127.0.0.1, adds both, and streams
//! byte ranges through unbuffered; it is the server's playback proxy, minus the
//! trip home.
//!
//! Each registered stream gets an unguessable path, so another app on the
//! phone that finds the port cannot use it as an open proxy.
//!
//! The same relay takes exports the other way (`video_export`): the page
//! `PUT`s a file in chunks to a registered upload, and the relay writes them
//! into the file Rust opened for it. A page cannot write a file on Android, and
//! chunked requests carry hundreds of megabytes that the WebView bridge's
//! strings could not.

use std::{
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    io::Write,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, tcp::OwnedReadHalf},
};

use crate::models::PlaybackRequestHeader;

/// How long a registered stream is served. YouTube's own URLs last about six
/// hours; a stream nobody has asked for in that long is not coming back.
const TARGET_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);

/// A wedged upstream has to surface as an error the player can recover from,
/// not hang a fetch forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const CHUNK_TIMEOUT: Duration = Duration::from_secs(20);

/// The longest request head accepted. Shaka's are a few hundred bytes.
const MAX_HEAD: usize = 16 * 1024;

/// The largest chunk of an upload. The page sends 8 MB at a time.
const MAX_UPLOAD_CHUNK: usize = 32 * 1024 * 1024;

/// A file being written by the page, chunk by chunk, in order.
struct Upload {
    /// Taken when the upload is finished, which closes the file.
    file: Option<std::fs::File>,
    written: u64,
}

type Uploads = Arc<Mutex<HashMap<String, Arc<Mutex<Upload>>>>>;

#[derive(Clone)]
struct Target {
    url: String,
    headers: Vec<PlaybackRequestHeader>,
    registered: Instant,
}

pub struct LocalRelay {
    port: u16,
    targets: Arc<Mutex<HashMap<String, Target>>>,
    uploads: Uploads,
    counter: AtomicU64,
    secret: u64,
}

static RELAY: OnceLock<Result<LocalRelay, String>> = OnceLock::new();

/// The relay, started on first use.
pub fn relay() -> Result<&'static LocalRelay, String> {
    RELAY
        .get_or_init(LocalRelay::start)
        .as_ref()
        .map_err(Clone::clone)
}

impl LocalRelay {
    /// Binds a free port and serves it from a runtime of its own, so the relay
    /// does not depend on whichever runtime the UI happens to run on.
    fn start() -> Result<Self, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("The local relay could not listen: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("The local relay could not listen: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("The local relay has no address: {error}"))?
            .port();
        let targets = Arc::new(Mutex::new(HashMap::new()));
        let served = targets.clone();
        let uploads: Uploads = Arc::new(Mutex::new(HashMap::new()));
        let written = uploads.clone();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tawny-relay")
            .enable_all()
            .build()
            .map_err(|error| format!("The local relay could not start: {error}"))?;
        std::thread::Builder::new()
            .name("tawny-relay-accept".into())
            .spawn(move || runtime.block_on(accept(listener, served, written)))
            .map_err(|error| format!("The local relay could not start: {error}"))?;
        Ok(Self {
            port,
            targets,
            uploads,
            counter: AtomicU64::new(1),
            secret: std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish(),
        })
    }

    /// Serve `url`, fetched with `headers`, and answer the address to give the
    /// player instead.
    pub fn register(&self, url: String, headers: Vec<PlaybackRequestHeader>) -> String {
        let token = self.token();
        if let Ok(mut targets) = self.targets.lock() {
            targets.retain(|_, target| target.registered.elapsed() < TARGET_LIFETIME);
            targets.insert(
                token.clone(),
                Target {
                    url,
                    headers,
                    registered: Instant::now(),
                },
            );
        }
        format!("http://127.0.0.1:{}/s/{token}", self.port)
    }

    /// Take chunks for `file` at the address answered, until
    /// [`Self::finish_upload`] with the same address.
    pub fn register_upload(&self, file: std::fs::File) -> String {
        let token = self.token();
        if let Ok(mut uploads) = self.uploads.lock() {
            uploads.insert(
                token.clone(),
                Arc::new(Mutex::new(Upload {
                    file: Some(file),
                    written: 0,
                })),
            );
        }
        format!("http://127.0.0.1:{}/u/{token}", self.port)
    }

    /// Stop taking chunks for the upload at `address` and close its file,
    /// answering how many bytes it holds. Waits for a chunk being written.
    pub fn finish_upload(&self, address: &str) -> Result<u64, String> {
        let token = address.rsplit('/').next().unwrap_or_default();
        let upload = self
            .uploads
            .lock()
            .ok()
            .and_then(|mut uploads| uploads.remove(token))
            .ok_or("The export was not being written")?;
        let mut upload = upload
            .lock()
            .map_err(|_| "The export stopped while it was written".to_string())?;
        let file = upload
            .file
            .take()
            .ok_or("The export was already finished")?;
        file.sync_all()
            .map_err(|error| format!("Could not save the export: {error}"))?;
        Ok(upload.written)
    }

    fn token(&self) -> String {
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(self.secret ^ count);
        format!("{:016x}{count:x}", hasher.finish())
    }
}

async fn accept(
    listener: std::net::TcpListener,
    targets: Arc<Mutex<HashMap<String, Target>>>,
    uploads: Uploads,
) {
    let Ok(listener) = TcpListener::from_std(listener) else {
        return;
    };
    let Some(client) = client() else {
        return;
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let targets = targets.clone();
        let uploads = uploads.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let _ = serve(stream, &targets, &uploads, &client).await;
        });
    }
}

/// The client that fetches from YouTube, trusting the bundled Mozilla roots.
///
/// reqwest 0.13 otherwise verifies through rustls-platform-verifier, which on
/// Android needs a JNI context and its Kotlin component set up first; the app
/// does neither, and the first handshake would panic. The bundled roots are
/// what reqwest 0.12's `rustls-tls` used, so behaviour is unchanged.
fn client() -> Option<reqwest::Client> {
    let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|cert| reqwest::Certificate::from_der(cert).ok());
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .tls_certs_only(roots)
        .build()
        .ok()
}

/// One request per connection: the player is on the same device, so a new
/// connection costs nothing, and closing after each response keeps the
/// parsing to a request head.
async fn serve(
    stream: TcpStream,
    targets: &Mutex<HashMap<String, Target>>,
    uploads: &Uploads,
    client: &reqwest::Client,
) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let Some((request, mut body)) = read_head(reader).await? else {
        return Ok(());
    };
    if request.method == "OPTIONS" {
        return writer
            .write_all(
                format!("HTTP/1.1 204 No Content\r\n{CORS}Content-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await;
    }
    if request.method == "PUT" {
        let status = receive_chunk(&request, &mut body, uploads).await;
        return respond(&mut writer, status).await;
    }
    if request.method != "GET" && request.method != "HEAD" {
        return respond(&mut writer, "405 Method Not Allowed").await;
    }
    let target = request
        .path
        .strip_prefix("/s/")
        .and_then(|token| targets.lock().ok()?.get(token).cloned());
    let Some(target) = target else {
        return respond(&mut writer, "404 Not Found").await;
    };

    let mut upstream = client.get(&target.url);
    for header in &target.headers {
        upstream = upstream.header(&header.name, &header.value);
    }
    if let Some(range) = &request.range {
        upstream = upstream.header("Range", range);
    }
    let mut response = match upstream.send().await {
        Ok(response) => response,
        Err(_) => return respond(&mut writer, "502 Bad Gateway").await,
    };

    let status = response.status();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n{CORS}Connection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
    ] {
        if let Some(value) = response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
        {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes()).await?;
    if request.method == "HEAD" {
        return Ok(());
    }
    // Written as it arrives. Holding a range back until it was whole starved
    // the player's bandwidth estimate on the server's proxy once; the same
    // holds here.
    loop {
        match tokio::time::timeout(CHUNK_TIMEOUT, response.chunk()).await {
            Ok(Ok(Some(chunk))) => writer.write_all(&chunk).await?,
            Ok(Ok(None)) => break,
            // The connection closes short of Content-Length, which the
            // player sees as a failed request and retries.
            Ok(Err(_)) | Err(_) => break,
        }
    }
    writer.shutdown().await
}

/// One chunk of an upload, written where the page says it goes. A chunk
/// that does not start where the last one ended is refused rather than
/// written, so a retried or reordered request cannot corrupt the file.
async fn receive_chunk(
    request: &RequestHead,
    body: &mut BufReader<OwnedReadHalf>,
    uploads: &Uploads,
) -> &'static str {
    let (path, query) = request
        .path
        .split_once('?')
        .unwrap_or((request.path.as_str(), ""));
    let upload = path
        .strip_prefix("/u/")
        .and_then(|token| uploads.lock().ok()?.get(token).cloned());
    let Some(upload) = upload else {
        return "404 Not Found";
    };
    let Some(offset) = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("offset="))
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return "400 Bad Request";
    };
    let Some(length) = request
        .content_length
        .filter(|length| *length <= MAX_UPLOAD_CHUNK)
    else {
        return "413 Content Too Large";
    };
    let mut chunk = vec![0; length];
    if body.read_exact(&mut chunk).await.is_err() {
        return "400 Bad Request";
    }
    // Written off the relay's threads: a phone's storage can stall for longer
    // than a request should hold one.
    tokio::task::spawn_blocking(move || {
        let Ok(mut upload) = upload.lock() else {
            return "500 Internal Server Error";
        };
        if offset != upload.written {
            return "409 Conflict";
        }
        let Some(file) = upload.file.as_mut() else {
            return "410 Gone";
        };
        if file.write_all(&chunk).is_err() {
            return "507 Insufficient Storage";
        }
        upload.written += chunk.len() as u64;
        "204 No Content"
    })
    .await
    .unwrap_or("500 Internal Server Error")
}

/// What the WebView needs to accept a response for a page on another origin.
const CORS: &str = "Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, HEAD, PUT, OPTIONS\r\n\
Access-Control-Allow-Headers: range, content-type\r\n\
Access-Control-Expose-Headers: content-length, content-range, accept-ranges\r\n\
Access-Control-Max-Age: 86400\r\n";

async fn respond(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
) -> std::io::Result<()> {
    writer
        .write_all(
            format!("HTTP/1.1 {status}\r\n{CORS}Content-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
}

struct RequestHead {
    method: String,
    path: String,
    range: Option<String>,
    content_length: Option<usize>,
}

/// The request head, and the reader positioned at its body.
async fn read_head(
    reader: OwnedReadHalf,
) -> std::io::Result<Option<(RequestHead, BufReader<OwnedReadHalf>)>> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut total = 0;
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let mut head = RequestHead {
        method: method.to_string(),
        path: path.to_string(),
        range: None,
        content_length: None,
    };
    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        total += read;
        if read == 0 || total > MAX_HEAD || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("range") {
                head.range = Some(value.trim().to_string());
            } else if name.eq_ignore_ascii_case("content-length") {
                head.content_length = value.trim().parse().ok();
            }
        }
    }
    Ok(Some((head, reader)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for YouTube: serves `BODY`, honouring one byte range, and
    /// remembers the user agent it was asked with.
    async fn upstream() -> (String, Arc<Mutex<Option<String>>>) {
        const BODY: &[u8] = b"0123456789abcdefghij";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(None));
        let remembered = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 4096];
                let read = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let agent = request
                    .lines()
                    .find_map(|line| line.strip_prefix("user-agent: "))
                    .map(str::to_string);
                *remembered.lock().unwrap() = agent;
                let range = request.lines().find_map(|line| {
                    let (start, end) = line.strip_prefix("range: bytes=")?.split_once('-')?;
                    Some((
                        start.parse::<usize>().ok()?,
                        end.trim().parse::<usize>().ok()?,
                    ))
                });
                let response = match range {
                    Some((start, end)) => {
                        let slice = &BODY[start..=end];
                        format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-type: video/mp4\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{end}/{}\r\n\r\n{}",
                            slice.len(),
                            BODY.len(),
                            String::from_utf8_lossy(slice)
                        )
                    }
                    None => format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
                        BODY.len(),
                        String::from_utf8_lossy(BODY)
                    ),
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{address}/videoplayback"), seen)
    }

    #[tokio::test]
    async fn ranges_come_through_with_cors_and_the_resolved_headers() {
        let (url, seen) = upstream().await;
        let relay = relay().unwrap();
        let address = relay.register(
            url,
            vec![PlaybackRequestHeader {
                name: "User-Agent".into(),
                value: "resolved-agent".into(),
            }],
        );
        let response = reqwest::Client::new()
            .get(&address)
            .header("Range", "bytes=2-5")
            .header("Origin", "https://dioxus.index.html")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "*",
            "the WebView refuses a cross-origin response without it"
        );
        assert_eq!(response.headers()["content-range"], "bytes 2-5/20");
        assert_eq!(response.text().await.unwrap(), "2345");
        assert_eq!(seen.lock().unwrap().as_deref(), Some("resolved-agent"));
    }

    #[tokio::test]
    async fn a_preflight_is_answered_and_unknown_paths_are_not_served() {
        let relay = relay().unwrap();
        let base = relay.register("http://127.0.0.1:9/never".into(), Vec::new());
        let client = reqwest::Client::new();
        let preflight = client
            .request(reqwest::Method::OPTIONS, &base)
            .send()
            .await
            .unwrap();
        assert_eq!(preflight.status(), 204);
        assert_eq!(
            preflight.headers()["access-control-allow-headers"],
            "range, content-type"
        );

        let guessed = base.rsplit_once('/').unwrap().0.to_string() + "/0";
        assert_eq!(client.get(guessed).send().await.unwrap().status(), 404);
    }

    #[tokio::test]
    async fn uploads_are_written_in_order_and_only_in_order() {
        let path = std::env::temp_dir().join(format!("tawny-upload-{}.bin", std::process::id()));
        let relay = relay().unwrap();
        let address = relay.register_upload(std::fs::File::create(&path).unwrap());
        let client = reqwest::Client::new();
        let put = |offset: u64, body: &'static [u8]| {
            client
                .put(format!("{address}?offset={offset}"))
                .header("Origin", "https://dioxus.index.html")
                .body(body)
                .send()
        };

        let first = put(0, b"hello ").await.unwrap();
        assert_eq!(first.status(), 204);
        assert_eq!(first.headers()["access-control-allow-origin"], "*");
        assert_eq!(
            put(0, b"again").await.unwrap().status(),
            409,
            "a repeated chunk would duplicate bytes"
        );
        assert_eq!(put(6, b"world").await.unwrap().status(), 204);
        assert_eq!(relay.finish_upload(&address).unwrap(), 11);
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
        assert_eq!(put(11, b"!").await.unwrap().status(), 404, "finished");
        let _ = std::fs::remove_file(&path);
    }
}
