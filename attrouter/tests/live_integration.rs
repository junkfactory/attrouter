//! Live-router integration tests.
//!
//! These tests drive `attrouter::Client` through a [`ProtectiveProxy`]
//! that sits in front of a real AT&T gateway. The proxy's default mode
//! intercepts destructive POSTs (reboot, broadband restart, wifi
//! restart, factory reset, clear-and-rescan device list) so the tests
//! are safe to run against a production router.
//!
//! The gateway is located via [`common::live_router_upstream`], which
//! honours `LIVE_ROUTER` if set and otherwise auto-discovers the AT&T
//! gateway on the local network. Tests skip silently on non-AT&T
//! networks. Destructive tests hard-fail instead of skip — opting into
//! [`DestructiveMode::Transparent`] is an explicit intent, and the
//! absence of a reachable router in that mode is an error.
//!
//! # Environment
//!
//! - `LIVE_ROUTER=192.168.1.254` — optional. Overrides auto-discovery.
//! - `LIVE_ROUTER_ACCESS_CODE=ABCD1234EFGH` — required for tests that
//!   authenticate (login, intercept-reboot).
//! - `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1` — opt in to
//!   `DestructiveMode::Transparent`. Only set this if you are OK with
//!   the router actually being rebooted during a test run.
//!
//! # Running
//!
//! ```sh
//! # safe: reads only, reboot is intercepted
//! LIVE_ROUTER=192.168.1.254 \
//! LIVE_ROUTER_ACCESS_CODE=xxxxxxxxxxxx \
//!   cargo test --test live_integration
//!
//! # unsafe: actually reboots the router, run serially
//! LIVE_ROUTER=192.168.1.254 \
//! LIVE_ROUTER_ACCESS_CODE=xxxxxxxxxxxx \
//! LIVE_ROUTER_ALLOW_DESTRUCTIVE=1 \
//!   cargo test --test live_integration -- --test-threads=1
//! ```

mod common;

use common::{
    DestructiveMode, ProtectiveProxy, client_for, live_access_code, live_destructive_allowed,
    live_router_upstream,
};

/// Skip the test (with an eprintln!) unless a live AT&T gateway is
/// reachable — either via `LIVE_ROUTER` env or auto-discovery. Returns
/// the upstream base URL when enabled.
macro_rules! require_live {
    () => {{
        match live_router_upstream().await {
            Some(u) => u,
            None => {
                eprintln!(
                    "skipping: no AT&T gateway found (set LIVE_ROUTER=<ip> to override \
                     auto-discovery, or connect to an AT&T-served network)"
                );
                return;
            }
        }
    }};
}

/// Like `require_live!` but also requires an access code. Useful for
/// tests that need to authenticate.
macro_rules! require_live_with_auth {
    () => {{
        let upstream = require_live!();
        let Some(code) = live_access_code() else {
            eprintln!("skipping: set LIVE_ROUTER_ACCESS_CODE to enable this test");
            return;
        };
        (upstream, code)
    }};
}

#[tokio::test]
async fn live_sysinfo_returns_real_model_number() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    let info = client.sysinfo().await.expect("sysinfo");
    assert!(
        info.model_number.is_some(),
        "real gateway should report a model number"
    );
    assert!(
        info.manufacturer.is_some(),
        "real gateway should report a manufacturer"
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_broadband_is_up_and_has_ipv4() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    let bb = client.broadband_status().await.expect("broadband");
    assert!(
        bb.ipv4_address.is_some(),
        "live gateway should have a WAN IPv4"
    );
    // connection status: "Up" or similar
    assert!(
        bb.connection.is_some(),
        "live gateway should report a connection state"
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_lan_has_dhcp_range() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    let lan = client.lan_status().await.expect("lan");
    assert!(lan.dhcpv4_start.is_some());
    assert!(lan.dhcpv4_end.is_some());
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_devices_returns_at_least_one_entry() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    let list = client.devices().await.expect("devices");
    // The host running the tests must itself be on the LAN, so the
    // gateway should know about at least one device.
    assert!(!list.is_empty(), "expected at least one device");
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_login_succeeds_with_access_code() {
    let (upstream, code) = require_live_with_auth!();
    // Login POST is non-destructive and flows through the proxy.
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    client
        .login(&code)
        .await
        .expect("login against live router");
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_reboot_is_intercepted_by_default() {
    let (upstream, code) = require_live_with_auth!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    client.login(&code).await.expect("login");

    // The Client thinks this succeeded, but the proxy absorbed the POST.
    client.restart_device().await.expect("reboot (intercepted)");

    // At least one destructive POST was intercepted. There may be more
    // — `restart_device` fetches the restart page first via GET, then
    // POSTs; only the POST is intercepted.
    assert!(
        proxy.intercepted_count() >= 1,
        "expected at least one intercepted destructive POST, got {}",
        proxy.intercepted_count()
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_clear_devices_is_intercepted_by_default() {
    let (upstream, code) = require_live_with_auth!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;
    let client = client_for(&proxy.base_url());
    client.login(&code).await.expect("login");
    client
        .clear_device_list()
        .await
        .expect("clear (intercepted)");
    assert!(proxy.intercepted_count() >= 1);
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_proxy_forwards_safe_gets() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;

    // Raw GET bypassing the Client — the proxy should forward to the
    // live router and return its response verbatim.
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/cgi-bin/sysinfo.ha", proxy.base_url()))
        .send()
        .await
        .expect("forwarded GET");
    assert!(resp.status().is_success());
    assert!(proxy.forwarded_count() >= 1);
    assert_eq!(proxy.intercepted_count(), 0);

    let body = resp.text().await.unwrap();
    assert!(body.contains("Model Number"));
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_proxy_intercepts_raw_destructive_post() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Block).await;

    // Raw POST to the destructive restart endpoint. The proxy must
    // intercept and return the canned acceptance page without ever
    // forwarding anything upstream.
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/cgi-bin/restart.ha", proxy.base_url()))
        .form(&[("nonce", "dummy"), ("Restart", "Restart")])
        .send()
        .await
        .expect("raw POST");
    assert!(resp.status().is_success());
    assert_eq!(
        proxy.intercepted_count(),
        1,
        "proxy should have intercepted the destructive POST"
    );
    assert_eq!(
        proxy.forwarded_count(),
        0,
        "no upstream traffic should happen for an intercepted POST"
    );

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Restart in Progress"),
        "response should be the canned restart_accepted.html fixture"
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_transparent_proxy_does_not_intercept() {
    let upstream = require_live!();
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Transparent).await;

    // A raw GET should still flow through; the intercept counter stays zero.
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/cgi-bin/sysinfo.ha", proxy.base_url()))
        .send()
        .await
        .expect("forwarded GET");
    assert!(resp.status().is_success());
    assert_eq!(proxy.intercepted_count(), 0);
    assert!(proxy.forwarded_count() >= 1);
    proxy.shutdown().await;
}

#[tokio::test]
async fn live_reboot_actually_reboots_when_opted_in() {
    if !live_destructive_allowed() {
        eprintln!("skipping: set LIVE_ROUTER_ALLOW_DESTRUCTIVE=1 to actually reboot the router");
        return;
    }
    // Destructive mode is an explicit opt-in; a missing router at this
    // point is an error, not a skip condition. `expect` turns either
    // failure into a loud test panic with a clear message.
    let upstream = live_router_upstream().await.expect(
        "destructive live test requires a reachable AT&T gateway — set LIVE_ROUTER=<ip> \
         or run on an AT&T-served network",
    );
    let code = live_access_code()
        .expect("destructive live test requires LIVE_ROUTER_ACCESS_CODE to be set");
    let proxy = ProtectiveProxy::spawn(upstream, DestructiveMode::Transparent).await;
    let client = client_for(&proxy.base_url());
    client.login(&code).await.expect("login");
    // This call will drop the connection and the router will take
    // 60-120s to come back. The intercepted counter must stay zero.
    let _ = client.restart_device().await;
    assert_eq!(proxy.intercepted_count(), 0);
    proxy.shutdown().await;
}
