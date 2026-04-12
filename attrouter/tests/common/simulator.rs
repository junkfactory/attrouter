//! `FakeGateway` — an axum-based simulator for the AT&T residential
//! gateway web UI.
//!
//! Binds to `127.0.0.1:0` (ephemeral port) and serves sanitized captures
//! of the BGW320 pages with fresh nonces per render. Tracks per-session
//! state (cookie + authentication + last-issued nonce) so the mutating
//! endpoints can be exercised end-to-end without ever touching real
//! hardware.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Form, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use md5::{Digest, Md5};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::fixtures;

/// A device the simulator will render into its devices.ha table.
#[derive(Debug, Clone)]
pub struct SimDevice {
    pub mac: String,
    pub name: Option<String>,
    pub ipv4: Option<String>,
    pub status: String,
    pub connection_type: String,
}

impl SimDevice {
    pub fn new(mac: &str, name: &str, ipv4: &str) -> Self {
        Self {
            mac: mac.into(),
            name: Some(name.into()),
            ipv4: Some(ipv4.into()),
            status: "on".into(),
            connection_type: "Ethernet LAN-1".into(),
        }
    }
}

#[derive(Default)]
struct Session {
    authenticated: bool,
    /// Most recently issued form nonce for this session. Any POST from
    /// this session must echo this value to be accepted.
    last_nonce: Option<String>,
}

struct State_ {
    access_code: String,
    sessions: HashMap<String, Session>,
    devices: Vec<SimDevice>,
    reboots_requested: u32,
    broadband_restarts: u32,
    wifi_restarts: [u32; 2],
    clear_devices_count: u32,
}

impl State_ {
    fn new() -> Self {
        Self {
            access_code: "letmein".into(),
            sessions: HashMap::new(),
            devices: default_devices(),
            reboots_requested: 0,
            broadband_restarts: 0,
            wifi_restarts: [0, 0],
            clear_devices_count: 0,
        }
    }
}

fn default_devices() -> Vec<SimDevice> {
    vec![
        SimDevice::new("00:11:22:33:44:56", "host-alpha", "192.168.1.70"),
        SimDevice::new("00:11:22:33:44:59", "host-gamma", "192.168.1.74"),
    ]
}

#[derive(Clone)]
struct SharedState(Arc<Mutex<State_>>);

/// Handle returned from [`FakeGateway::spawn`]. Dropping the handle
/// without calling [`FakeGateway::shutdown`] will cancel the background
/// task on process exit but not gracefully.
pub struct FakeGateway {
    addr: SocketAddr,
    state: SharedState,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl FakeGateway {
    /// Spawn a fake gateway on an ephemeral `127.0.0.1` port. Returns
    /// once the socket is bound and ready to accept connections.
    pub async fn spawn() -> Self {
        let state = SharedState(Arc::new(Mutex::new(State_::new())));
        let app = Router::new()
            .route("/", get(root_redirect))
            .route("/cgi-bin/index.ha", get(root_redirect))
            .route("/cgi-bin/sysinfo.ha", get(handle_sysinfo))
            .route("/cgi-bin/broadbandstatistics.ha", get(handle_broadband))
            .route("/cgi-bin/lanstatistics.ha", get(handle_lan))
            .route(
                "/cgi-bin/devices.ha",
                get(handle_devices_get).post(handle_devices_post),
            )
            .route("/cgi-bin/home.ha", get(handle_home))
            .route(
                "/cgi-bin/login.ha",
                get(handle_login_get).post(handle_login_post),
            )
            .route(
                "/cgi-bin/restart.ha",
                get(handle_restart_get).post(handle_restart_post),
            )
            .route("/cgi-bin/crestart.ha", post(handle_crestart))
            .route("/cgi-bin/wrestart.ha", post(handle_wrestart))
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

    /// Base URL suitable for `Client::with_base_url`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Gracefully stop the simulator.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }

    pub fn set_access_code(&self, code: &str) {
        self.state.0.lock().unwrap().access_code = code.into();
    }

    pub fn set_devices(&self, devices: Vec<SimDevice>) {
        self.state.0.lock().unwrap().devices = devices;
    }

    pub fn reboots_requested(&self) -> u32 {
        self.state.0.lock().unwrap().reboots_requested
    }

    pub fn broadband_restarts(&self) -> u32 {
        self.state.0.lock().unwrap().broadband_restarts
    }

    pub fn wifi_restarts(&self) -> [u32; 2] {
        self.state.0.lock().unwrap().wifi_restarts
    }

    pub fn clear_devices_count(&self) -> u32 {
        self.state.0.lock().unwrap().clear_devices_count
    }
}

impl Drop for FakeGateway {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// --- session helpers ------------------------------------------------------

fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .map(|s| s.trim())
        .find(|c| c.starts_with("SessionID="))
        .map(|c| c.trim_start_matches("SessionID=").to_string())
}

fn new_session_id() -> String {
    fixtures::fresh_nonce()
}

/// Ensure a session exists for this request; return its ID plus a flag
/// indicating whether it was just created (so the response can attach a
/// `Set-Cookie` header).
fn ensure_session(state: &SharedState, headers: &HeaderMap) -> (String, bool) {
    let mut st = state.0.lock().unwrap();
    if let Some(sid) = extract_session_id(headers)
        && st.sessions.contains_key(&sid)
    {
        return (sid, false);
    }
    let sid = new_session_id();
    st.sessions.insert(sid.clone(), Session::default());
    (sid, true)
}

fn issue_nonce(state: &SharedState, session_id: &str) -> String {
    let nonce = fixtures::fresh_nonce();
    let mut st = state.0.lock().unwrap();
    if let Some(s) = st.sessions.get_mut(session_id) {
        s.last_nonce = Some(nonce.clone());
    }
    nonce
}

fn is_authenticated(state: &SharedState, session_id: &str) -> bool {
    state
        .0
        .lock()
        .unwrap()
        .sessions
        .get(session_id)
        .map(|s| s.authenticated)
        .unwrap_or(false)
}

fn consume_nonce(state: &SharedState, session_id: &str, submitted: &str) -> bool {
    let mut st = state.0.lock().unwrap();
    let Some(session) = st.sessions.get_mut(session_id) else {
        return false;
    };
    match session.last_nonce.as_deref() {
        Some(expected) if expected == submitted => {
            // Invalidate the nonce so it can't be replayed. A real
            // request cycle would fetch a fresh form page (and thus a
            // fresh nonce) before the next POST.
            session.last_nonce = None;
            true
        }
        _ => false,
    }
}

/// Wrap an HTML body with a 200 response and the `Set-Cookie` session
/// header (if newly issued).
fn html_response(body: String, session_id: &str, set_cookie: bool) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=windows-1252"),
    );
    if set_cookie {
        let cookie = format!("SessionID={session_id}; HttpOnly; Path=/");
        headers.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    }
    (StatusCode::OK, headers, body).into_response()
}

fn redirect(location: &str, session_id: &str, set_cookie: bool) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, HeaderValue::from_str(location).unwrap());
    if set_cookie {
        let cookie = format!("SessionID={session_id}; HttpOnly; Path=/");
        headers.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    }
    (StatusCode::FOUND, headers, ()).into_response()
}

// --- handlers -------------------------------------------------------------

async fn root_redirect(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    redirect("/cgi-bin/home.ha", &sid, new)
}

async fn handle_sysinfo(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    html_response(fixtures::SYSINFO.to_string(), &sid, new)
}

async fn handle_broadband(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    html_response(fixtures::render(fixtures::BROADBAND, &nonce), &sid, new)
}

async fn handle_lan(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    html_response(fixtures::render(fixtures::LAN, &nonce), &sid, new)
}

async fn handle_home(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    html_response(fixtures::render(fixtures::HOME, &nonce), &sid, new)
}

async fn handle_devices_get(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    let body = render_devices_page(&state, &nonce);
    html_response(body, &sid, new)
}

async fn handle_devices_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    if !is_authenticated(&state, &sid) {
        // Mirror the real router: render the login page at the same URL.
        let nonce = issue_nonce(&state, &sid);
        return html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new);
    }
    let submitted = form.get("nonce").cloned().unwrap_or_default();
    if !consume_nonce(&state, &sid, &submitted) {
        return (StatusCode::BAD_REQUEST, "stale nonce").into_response();
    }
    {
        let mut st = state.0.lock().unwrap();
        st.clear_devices_count += 1;
        st.devices.clear();
    }
    let nonce = issue_nonce(&state, &sid);
    html_response(render_devices_page(&state, &nonce), &sid, new)
}

fn render_devices_page(state: &SharedState, nonce: &str) -> String {
    // Rather than splicing dynamic rows into the captured fixture, we
    // substitute a freshly-generated device table into a minimal
    // template. This is good enough for the existing parser tests
    // because they only care about MAC / IPv4 / Name rows and the
    // `<hr class="reshr"/>` separators.
    let mut body = String::new();
    body.push_str(r#"<!DOCTYPE html><html><head><title>Device List</title></head><body>"#);
    body.push_str(&format!(
        r#"<form method="post" action="/cgi-bin/devices.ha"><input type="hidden" name="nonce" value="{nonce}"/>"#
    ));
    body.push_str(r#"<table class="table100" summary="devices">"#);

    let st = state.0.lock().unwrap();
    let count = st.devices.len();
    for (i, d) in st.devices.iter().enumerate() {
        body.push_str(&format!(
            r#"<tr><th scope="row">MAC Address</th><td class="col2">{}</td></tr>"#,
            d.mac
        ));
        if let (Some(ip), Some(name)) = (&d.ipv4, &d.name) {
            body.push_str(&format!(
                r#"<tr><th scope="row">IPv4 Address / Name</th><td class="col2">{ip} / {name}</td></tr>"#,
            ));
        } else if let Some(name) = &d.name {
            body.push_str(&format!(
                r#"<tr><th scope="row">Name</th><td class="col2">{name}</td></tr>"#,
            ));
        }
        body.push_str(&format!(
            r#"<tr><th scope="row">Status</th><td class="col2">{}</td></tr>"#,
            d.status
        ));
        body.push_str(&format!(
            r#"<tr><th scope="row">Connection Type</th><td class="col2">{}</td></tr>"#,
            d.connection_type
        ));
        body.push_str(r#"<tr><th scope="row">Mesh Client</th><td class="col2">No</td></tr>"#);
        if i + 1 < count {
            body.push_str(r#"<tr><td colspan="2"><hr class="reshr" noshade="noshade"/></td></tr>"#);
        }
    }
    body.push_str("</table></form></body></html>");
    body
}

async fn handle_login_get(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new)
}

async fn handle_login_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let submitted_nonce = form.get("nonce").cloned().unwrap_or_default();
    let hashpassword = form.get("hashpassword").cloned().unwrap_or_default();

    let access_code = state.0.lock().unwrap().access_code.clone();
    let mut hasher = Md5::new();
    hasher.update(access_code.as_bytes());
    hasher.update(submitted_nonce.as_bytes());
    let expected = hex::encode(hasher.finalize());

    // Validate nonce via consume_nonce (mirrors real CSRF behaviour).
    if !consume_nonce(&state, &sid, &submitted_nonce) {
        let nonce = issue_nonce(&state, &sid);
        return html_response(
            fixtures::render(fixtures::LOGIN_REJECTED, &nonce),
            &sid,
            new,
        );
    }

    if hashpassword == expected {
        state
            .0
            .lock()
            .unwrap()
            .sessions
            .get_mut(&sid)
            .unwrap()
            .authenticated = true;
        redirect("/cgi-bin/home.ha", &sid, new)
    } else {
        let nonce = issue_nonce(&state, &sid);
        html_response(
            fixtures::render(fixtures::LOGIN_REJECTED, &nonce),
            &sid,
            new,
        )
    }
}

async fn handle_restart_get(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    let nonce = issue_nonce(&state, &sid);
    if is_authenticated(&state, &sid) {
        html_response(fixtures::render(fixtures::RESTART, &nonce), &sid, new)
    } else {
        // Real router: renders login page directly at this URL.
        html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new)
    }
}

async fn handle_restart_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    if !is_authenticated(&state, &sid) {
        let nonce = issue_nonce(&state, &sid);
        return html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new);
    }
    let submitted = form.get("nonce").cloned().unwrap_or_default();
    if !consume_nonce(&state, &sid, &submitted) {
        return (StatusCode::BAD_REQUEST, "stale nonce").into_response();
    }
    state.0.lock().unwrap().reboots_requested += 1;
    html_response(fixtures::RESTART_ACCEPTED.to_string(), &sid, new)
}

async fn handle_crestart(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    if !is_authenticated(&state, &sid) {
        let nonce = issue_nonce(&state, &sid);
        return html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new);
    }
    let submitted = form.get("nonce").cloned().unwrap_or_default();
    if !consume_nonce(&state, &sid, &submitted) {
        return (StatusCode::BAD_REQUEST, "stale nonce").into_response();
    }
    state.0.lock().unwrap().broadband_restarts += 1;
    redirect("/cgi-bin/home.ha", &sid, new)
}

async fn handle_wrestart(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let (sid, new) = ensure_session(&state, &headers);
    if !is_authenticated(&state, &sid) {
        let nonce = issue_nonce(&state, &sid);
        return html_response(fixtures::render(fixtures::LOGIN, &nonce), &sid, new);
    }
    let submitted = form.get("nonce").cloned().unwrap_or_default();
    if !consume_nonce(&state, &sid, &submitted) {
        return (StatusCode::BAD_REQUEST, "stale nonce").into_response();
    }
    // Real router uses `?1` for 2.4 GHz and `?2` for 5 GHz; we accept
    // either form (raw "?1" vs "index=1") so tests stay loose.
    let idx = raw_query
        .as_deref()
        .map(|q| q.trim_start_matches('?'))
        .and_then(|q| q.parse::<usize>().ok())
        .unwrap_or(1);
    let bucket = if idx == 2 { 1 } else { 0 };
    state.0.lock().unwrap().wifi_restarts[bucket] += 1;
    redirect("/cgi-bin/home.ha", &sid, new)
}
