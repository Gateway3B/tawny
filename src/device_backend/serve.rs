//! The backend library's side: start the server on 127.0.0.1 and say where.

use std::{
    ffi::{CStr, c_char},
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
    sync::OnceLock,
};

use dioxus::server::axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::FutureExt;
use tokio::sync::watch;
use tower::ServiceExt;

use super::ABI_VERSION;
use crate::server::{self, AppServerState, DeviceBackend};

/// The port a device backend asks for first, so its address stays the same
/// from one launch to the next and the session cookie saved for it keeps
/// matching. When it is taken, any free port will do.
const PREFERRED_PORT: u16 = 48_080;

/// What the app checks before it calls [`tawny_backend_start`].
#[unsafe(no_mangle)]
pub extern "C" fn tawny_backend_abi() -> u32 {
    ABI_VERSION
}

/// Start the backend, keeping its data under `data_dir`. Returns the port it
/// listens on at 127.0.0.1, or 0 when it could not start.
///
/// It returns once the port is open, before the database is: requests that
/// arrive meanwhile wait for it. The app calls this before its first render,
/// and opening and migrating the database would hold that render up.
///
/// A second call returns the first call's port.
///
/// # Safety
///
/// `data_dir` is a NUL-terminated path, valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tawny_backend_start(data_dir: *const c_char) -> u16 {
    if data_dir.is_null() {
        return 0;
    }
    // SAFETY: the caller promises a valid NUL-terminated string.
    let data_dir = unsafe { CStr::from_ptr(data_dir) }
        .to_string_lossy()
        .into_owned();
    // A panic must not unwind across the C boundary into the app.
    std::panic::catch_unwind(|| start(PathBuf::from(data_dir)))
        .ok()
        .flatten()
        .unwrap_or(0)
}

fn start(data_dir: PathBuf) -> Option<u16> {
    static PORT: OnceLock<Option<u16>> = OnceLock::new();
    *PORT.get_or_init(|| {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PREFERRED_PORT))
            .or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
            .ok()?;
        listener.set_nonblocking(true).ok()?;
        let port = listener.local_addr().ok()?.port();
        // SurrealKit's schema sync reads and creates `database/setup.surql`
        // relative to the working directory, and nothing configures that. An
        // app's working directory is `/` on Android and its install folder on
        // a desktop, both read-only, so the backend works from its own data
        // folder. The app itself opens nothing by a relative path.
        std::fs::create_dir_all(&data_dir).ok()?;
        std::env::set_current_dir(&data_dir).ok()?;
        server::run_on_device(DeviceBackend {
            data_dir,
            public_url: format!("http://127.0.0.1:{port}"),
        });
        std::thread::Builder::new()
            .name("tawny-backend".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                    // A phone's cores are shared with the UI and the player.
                    .worker_threads(2)
                    .thread_name("tawny-backend")
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(serve(listener));
            })
            .ok()?;
        Some(port)
    })
}

#[derive(Clone)]
enum Startup {
    Opening,
    Ready(Router),
    /// Why, for the viewer. It stays on their own device, unlike a server's
    /// errors, which never leave it (see `api::server_error`).
    Failed(String),
}

async fn serve(listener: TcpListener) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        return;
    };
    let (ready, startup) = watch::channel(Startup::Opening);
    tokio::spawn(async move {
        let opening = async {
            match AppServerState::initialize().await {
                Ok(state) => {
                    server::spawn_subscription_poller(state.clone());
                    match crate::server_router::router(state).await {
                        Ok(router) => Startup::Ready(router),
                        Err(error) => Startup::Failed(format!("{error:#}")),
                    }
                }
                Err(error) => Startup::Failed(format!("{error:#}")),
            }
        };
        // A panic would otherwise leave every request waiting on a startup
        // that never reports.
        let outcome = std::panic::AssertUnwindSafe(opening)
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                    .unwrap_or_else(|| "it stopped unexpectedly".into());
                Startup::Failed(message)
            });
        let _ = ready.send(outcome);
    });

    let app = Router::new().fallback(move |request: Request<Body>| {
        let mut startup = startup.clone();
        async move {
            let state = startup
                .wait_for(|state| !matches!(state, Startup::Opening))
                .await
                .map(|state| state.clone());
            match state {
                Ok(Startup::Ready(router)) => router.oneshot(request).await.into_response(),
                Ok(Startup::Failed(reason)) => unavailable(&reason),
                _ => unavailable("it stopped"),
            }
        }
    });
    let _ = dioxus::server::axum::serve(listener, app).await;
}

/// The backend could not open its database. The client shows its setup
/// screen's error, and the viewer can choose a server instead.
fn unavailable(reason: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        format!("Tawny could not open its library on this device: {reason}"),
    )
        .into_response()
}
