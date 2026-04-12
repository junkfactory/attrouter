//! `ProtectiveProxy` — a test-only axum proxy that sits in front of a
//! real AT&T gateway and intercepts destructive POSTs by default.
//!
//! Used by `tests/live_integration.rs` so the same set of tests that
//! runs against [`super::simulator::FakeGateway`] can also run against a
//! real router without the risk of accidentally rebooting it every CI
//! run. Set `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1` to put the proxy into
//! `Transparent` mode and let every request through unchanged.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::fixtures;

/// Paths the proxy will intercept in [`DestructiveMode::Block`].
///
/// Match is by prefix so query strings are ignored.
const DESTRUCTIVE_PATHS: &[&str] = &[
    "/cgi-bin/restart.ha",
    "/cgi-bin/crestart.ha",
    "/cgi-bin/wrestart.ha",
    "/cgi-bin/reset.ha",
    // clear-and-rescan is listed because it disrupts the device list
    "/cgi-bin/devices.ha",
];

/// Whether the proxy should intercept destructive POSTs or forward
/// everything to the live router unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructiveMode {
    /// Default: intercept destructive POSTs and return canned responses.
    Block,
    /// Forward everything, including destructive operations. The live
    /// router will actually reboot (etc.) when this is set.
    Transparent,
}

#[derive(Clone)]
struct ProxyState {
    upstream: Arc<String>,
    mode: DestructiveMode,
    http: reqwest::Client,
    intercepted: Arc<AtomicU32>,
    forwarded: Arc<AtomicU32>,
}

pub struct ProtectiveProxy {
    addr: SocketAddr,
    state: ProxyState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl ProtectiveProxy {
    /// Spawn the proxy on an ephemeral `127.0.0.1` port, with `upstream`
    /// as the live router's base URL (e.g. `http://192.168.1.254`).
    pub async fn spawn(upstream: impl Into<String>, mode: DestructiveMode) -> Self {
        let http = reqwest::Client::builder()
            .cookie_store(false) // transparent — callers' cookies flow through
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        let state = ProxyState {
            upstream: Arc::new(upstream.into()),
            mode,
            http,
            intercepted: Arc::new(AtomicU32::new(0)),
            forwarded: Arc::new(AtomicU32::new(0)),
        };
        let app = Router::new()
            .fallback(any(forward_or_intercept))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Self {
            addr,
            state,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn intercepted_count(&self) -> u32 {
        self.state.intercepted.load(Ordering::SeqCst)
    }

    pub fn forwarded_count(&self) -> u32 {
        self.state.forwarded.load(Ordering::SeqCst)
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }
}

impl Drop for ProtectiveProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn path_is_destructive(path: &str) -> bool {
    DESTRUCTIVE_PATHS
        .iter()
        .any(|p| path == *p || path.starts_with(*p))
}

async fn forward_or_intercept(State(state): State<ProxyState>, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    // Destructive POST? Intercept and return canned success.
    if state.mode == DestructiveMode::Block && method == Method::POST && path_is_destructive(&path)
    {
        state.intercepted.fetch_add(1, Ordering::SeqCst);
        tracing::warn!(%path, "ProtectiveProxy intercepted destructive POST");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=windows-1252"),
        );
        return (StatusCode::OK, headers, fixtures::RESTART_ACCEPTED).into_response();
    }

    // Otherwise, forward upstream.
    state.forwarded.fetch_add(1, Ordering::SeqCst);
    match forward_upstream(&state, method, uri, req).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, "proxy upstream error");
            (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")).into_response()
        }
    }
}

async fn forward_upstream(
    state: &ProxyState,
    method: Method,
    uri: Uri,
    req: Request,
) -> Result<Response, reqwest::Error> {
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(uri.path());
    let url = format!("{}{}", state.upstream, path_and_query);

    // Copy headers except Host (reqwest will set it).
    let headers_in = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap_or_default();

    let rmethod =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut builder = state.http.request(rmethod, &url).body(body_bytes.to_vec());
    for (name, value) in headers_in.iter() {
        if name == header::HOST {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_bytes());
    }

    let upstream_resp = builder.send().await?;
    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();
    let body = upstream_resp.bytes().await?;
    Ok(build_response(status, headers, body))
}

fn build_response(
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: Bytes,
) -> Response {
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    let hdrs = resp.headers_mut();
    for (name, value) in headers.iter() {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            axum::http::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            hdrs.append(name, value);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_paths_are_recognized() {
        assert!(path_is_destructive("/cgi-bin/restart.ha"));
        assert!(path_is_destructive("/cgi-bin/crestart.ha"));
        assert!(path_is_destructive("/cgi-bin/wrestart.ha"));
        assert!(path_is_destructive("/cgi-bin/reset.ha"));
        assert!(path_is_destructive("/cgi-bin/devices.ha"));
    }

    #[test]
    fn safe_paths_are_not_destructive() {
        assert!(!path_is_destructive("/cgi-bin/sysinfo.ha"));
        assert!(!path_is_destructive("/cgi-bin/broadbandstatistics.ha"));
        assert!(!path_is_destructive("/cgi-bin/lanstatistics.ha"));
        assert!(!path_is_destructive("/cgi-bin/login.ha"));
        assert!(!path_is_destructive("/cgi-bin/home.ha"));
    }
}
