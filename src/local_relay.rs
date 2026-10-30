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

use std::{
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
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

#[derive(Clone)]
struct Target {
    url: String,
    headers: Vec<PlaybackRequestHeader>,
    registered: Instant,
}

pub struct LocalRelay {
    port: u16,
    targets: Arc<Mutex<HashMap<String, Target>>>,
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tawny-relay")
            .enable_all()
            .build()
            .map_err(|error| format!("The local relay could not start: {error}"))?;
        std::thread::Builder::new()
            .name("tawny-relay-accept".into())
            .spawn(move || runtime.block_on(accept(listener, served)))
            .map_err(|error| format!("The local relay could not start: {error}"))?;
        Ok(Self {
            port,
            targets,
            counter: AtomicU64::new(1),
            secret: std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish(),
        })
    }

    /// Serve `url`, fetched with `headers`, and answer the address to give the
    /// player instead.
    pub fn register(&self, url: String, headers: Vec<PlaybackRequestHeader>) -> String {
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(self.secret ^ count);
        let token = format!("{:016x}{count:x}", hasher.finish());
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
}

async fn accept(listener: std::net::TcpListener, targets: Arc<Mutex<HashMap<String, Target>>>) {
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
        let client = client.clone();
        tokio::spawn(async move {
            let _ = serve(stream, &targets, &client).await;
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
    client: &reqwest::Client,
) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let Some(request) = read_head(reader).await? else {
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

/// What the WebView needs to accept a response for a page on another origin.
const CORS: &str = "Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
Access-Control-Allow-Headers: range\r\n\
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
}

async fn read_head(reader: tokio::net::tcp::OwnedReadHalf) -> std::io::Result<Option<RequestHead>> {
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
    };
    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        total += read;
        if read == 0 || total > MAX_HEAD || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("range")
        {
            head.range = Some(value.trim().to_string());
        }
    }
    Ok(Some(head))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

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
        assert_eq!(preflight.headers()["access-control-allow-headers"], "range");

        let guessed = base.rsplit_once('/').unwrap().0.to_string() + "/0";
        assert_eq!(client.get(guessed).send().await.unwrap().status(), 404);
    }
}
