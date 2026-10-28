//! HTML fixtures baked into the test binary via `include_str!`.
//!
//! The simulator and the protective proxy both load their canned
//! responses from here. The pages are sanitized captures from a real
//! BGW320-500 (with MAC addresses, serial numbers, WAN IPs replaced);
//! form nonces are kept as the literal `{{NONCE}}` placeholder so the
//! simulator can substitute a fresh per-session nonce on every render.

pub const SYSINFO: &str = include_str!("../../src/testdata/sysinfo.html");
pub const BROADBAND: &str = include_str!("../../src/testdata/broadbandstatistics.html");
pub const LAN: &str = include_str!("../../src/testdata/lanstatistics.html");
pub const DEVICES: &str = include_str!("../../src/testdata/devices.html");

pub const HOME: &str = include_str!("fixtures/home.html");
pub const LOGIN: &str = include_str!("fixtures/login.html");
pub const LOGIN_NOTICE: &str = include_str!("fixtures/login_notice.html");
pub const LOGIN_REJECTED: &str = include_str!("fixtures/login_rejected.html");
pub const RESTART: &str = include_str!("fixtures/restart.html");
pub const RESTART_ACCEPTED: &str = include_str!("fixtures/restart_accepted.html");

/// Substitute every occurrence of `{{NONCE}}` in a fixture with a fresh
/// nonce value.
pub fn render(template: &str, nonce: &str) -> String {
    template.replace("{{NONCE}}", nonce)
}

#[allow(dead_code)]
pub fn fresh_nonce() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
