//! Shared test infrastructure for `attrouter`'s integration test suites.
//!
//! - [`simulator::FakeGateway`]: an in-memory axum server that mimics a
//!   BGW320 well enough to drive every `Client` method end-to-end.
//! - [`proxy::ProtectiveProxy`]: a forwarding axum server that sits in
//!   front of a real router, intercepting destructive POSTs so that the
//!   live router can be tested without ever actually being rebooted.
//!
//! Used by `tests/sim_integration.rs` and `tests/live_integration.rs`.

// Each `tests/*.rs` file is compiled as its own crate, so any helper
// not used by a particular test target shows up as dead-code / unused.
#![allow(dead_code, unused_imports)]

pub mod fixtures;
pub mod proxy;
pub mod simulator;

pub use proxy::{DestructiveMode, ProtectiveProxy};
pub use simulator::{FakeGateway, SimDevice};

/// Build a `Client` pointed at an arbitrary base URL (e.g. the address
/// reported by `FakeGateway::base_url()`).
pub fn client_for(base_url: &str) -> attrouter::Client {
    attrouter::Client::with_base_url(base_url).expect("build client")
}

/// Locate an AT&T gateway for live-integration tests to point at.
///
/// Resolution order:
///
/// 1. If `LIVE_ROUTER` is set, use it verbatim (env override wins so a
///    test run on an AT&T network can still point at a different host).
/// 2. Otherwise, call [`attrouter::discover`] to read the OS default
///    gateway and probe it for AT&T `sysinfo`. Returns `None` when no
///    AT&T-shaped device is reachable, so live tests skip silently on
///    non-AT&T networks.
///
/// The result is cached for the lifetime of the test binary via a
/// [`tokio::sync::OnceCell`] so every live-integration test in a single
/// `cargo test` invocation shares one discovery probe.
pub async fn live_router_upstream() -> Option<String> {
    static CACHED: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();
    CACHED
        .get_or_init(|| async {
            if let Ok(ip) = std::env::var("LIVE_ROUTER") {
                let url = if ip.starts_with("http://") || ip.starts_with("https://") {
                    ip
                } else {
                    format!("http://{ip}")
                };
                return Some(url);
            }
            match attrouter::discover().await {
                Ok(client) => Some(client.base_url().to_string()),
                Err(e) => {
                    tracing::debug!(error = %e, "live router auto-discovery failed");
                    None
                }
            }
        })
        .await
        .clone()
}

/// Read `LIVE_ROUTER_ACCESS_CODE`. Required for live tests that exercise
/// authenticated endpoints.
pub fn live_access_code() -> Option<String> {
    std::env::var("LIVE_ROUTER_ACCESS_CODE").ok()
}

/// True when the caller has opted in to destructive live-router tests
/// by setting `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1`. In this mode the
/// protective proxy runs in [`DestructiveMode::Transparent`] and
/// operations like `restart_device()` actually reboot the router.
pub fn live_destructive_allowed() -> bool {
    std::env::var("LIVE_ROUTER_ALLOW_DESTRUCTIVE").is_ok()
}
