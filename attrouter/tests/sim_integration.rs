//! End-to-end tests that drive `attrouter::Client` through an in-memory
//! [`FakeGateway`] (a.k.a. the simulator) running on an ephemeral
//! `127.0.0.1` port.
//!
//! These are the "layer 2" tests from the plan. They exercise the same
//! code paths the live integration tests do, but without any actual
//! network traffic and without touching a real router.

mod common;

use attrouter::{Error, WifiBand};
use common::{FakeGateway, SimDevice, client_for};

#[tokio::test]
async fn sysinfo_populates_expected_fields() {
    let sim = FakeGateway::spawn().await;
    let client = client_for(&sim.base_url());
    let info = client.sysinfo().await.expect("sysinfo");
    assert_eq!(info.manufacturer.as_deref(), Some("HUMAX"));
    assert_eq!(info.model_number.as_deref(), Some("BGW320-500"));
    assert_eq!(info.serial_number.as_deref(), Some("TEST1234567890"));
    assert_eq!(info.mac_address.as_deref(), Some("00:11:22:33:44:55"));
    sim.shutdown().await;
}

#[tokio::test]
async fn broadband_status_includes_ipv4_and_stats() {
    let sim = FakeGateway::spawn().await;
    let client = client_for(&sim.base_url());
    let bb = client.broadband_status().await.expect("broadband");
    assert_eq!(bb.ipv4_address.as_deref(), Some("192.0.2.1"));
    assert_eq!(bb.gateway_ipv4_address.as_deref(), Some("192.0.2.254"));
    assert_eq!(bb.mtu, Some(1500));
    assert_eq!(bb.current_speed_mbps, Some(10000));
    assert!(bb.ipv4_stats.receive_packets.is_some());
    sim.shutdown().await;
}

#[tokio::test]
async fn lan_status_has_dhcp_range_and_interfaces() {
    let sim = FakeGateway::spawn().await;
    let client = client_for(&sim.base_url());
    let lan = client.lan_status().await.expect("lan");
    assert_eq!(lan.device_ipv4_address.as_deref(), Some("192.168.1.254"));
    assert_eq!(lan.dhcpv4_start.as_deref(), Some("192.168.1.64"));
    assert_eq!(lan.dhcpv4_end.as_deref(), Some("192.168.1.253"));
    assert!(!lan.interfaces.is_empty());
    sim.shutdown().await;
}

#[tokio::test]
async fn devices_round_trip_from_simulated_state() {
    let sim = FakeGateway::spawn().await;
    sim.set_devices(vec![
        SimDevice::new("aa:bb:cc:dd:ee:01", "alpha", "10.0.0.1"),
        SimDevice::new("aa:bb:cc:dd:ee:02", "beta", "10.0.0.2"),
        SimDevice::new("aa:bb:cc:dd:ee:03", "gamma", "10.0.0.3"),
    ]);
    let client = client_for(&sim.base_url());
    let list = client.devices().await.expect("devices");
    assert_eq!(list.len(), 3);
    let macs: Vec<_> = list.iter().filter_map(|d| d.mac_address.clone()).collect();
    assert!(macs.contains(&"aa:bb:cc:dd:ee:01".to_string()));
    let names: Vec<_> = list.iter().filter_map(|d| d.name.clone()).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    sim.shutdown().await;
}

#[tokio::test]
async fn login_with_correct_code_succeeds() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    client.login("letmein").await.expect("login ok");
    sim.shutdown().await;
}

#[tokio::test]
async fn login_succeeds_after_cookie_notice() {
    let sim = FakeGateway::spawn().await;
    sim.set_cookie_notice(true);
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    client
        .login("letmein")
        .await
        .expect("login ok after cookie notice");
    let list = client.devices().await.expect("devices");
    assert!(!list.is_empty());
    sim.shutdown().await;
}

#[tokio::test]
async fn login_with_wrong_code_returns_auth_failed() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    let err = client.login("wrong").await.unwrap_err();
    assert!(matches!(err, Error::AuthFailed(_)), "got {err:?}");
    sim.shutdown().await;
}

#[tokio::test]
async fn restart_device_requires_prior_login() {
    let sim = FakeGateway::spawn().await;
    let client = client_for(&sim.base_url());
    let err = client.restart_device().await.unwrap_err();
    assert!(matches!(err, Error::NotAuthenticated), "got {err:?}");
    assert_eq!(sim.reboots_requested(), 0);
    sim.shutdown().await;
}

#[tokio::test]
async fn restart_device_after_login_increments_counter() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    client.login("letmein").await.expect("login");
    client.restart_device().await.expect("restart");
    assert_eq!(sim.reboots_requested(), 1);
    sim.shutdown().await;
}

#[tokio::test]
async fn restart_broadband_increments_counter() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    client.login("letmein").await.expect("login");
    client.restart_broadband().await.expect("restart broadband");
    assert_eq!(sim.broadband_restarts(), 1);
    sim.shutdown().await;
}

#[tokio::test]
async fn restart_wifi_2ghz_and_5ghz_increment_separately() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    let client = client_for(&sim.base_url());
    client.login("letmein").await.expect("login");
    client.restart_wifi(WifiBand::Ghz24).await.expect("2g");
    // home.ha fetch + POST happens for each call, so both get fresh nonces.
    client.restart_wifi(WifiBand::Ghz5).await.expect("5g");
    let [r24, r5] = sim.wifi_restarts();
    assert_eq!(r24, 1, "2.4 GHz restart count");
    assert_eq!(r5, 1, "5 GHz restart count");
    sim.shutdown().await;
}

#[tokio::test]
async fn clear_device_list_empties_state() {
    let sim = FakeGateway::spawn().await;
    sim.set_access_code("letmein");
    sim.set_devices(vec![
        SimDevice::new("aa:bb:cc:dd:ee:01", "alpha", "10.0.0.1"),
        SimDevice::new("aa:bb:cc:dd:ee:02", "beta", "10.0.0.2"),
    ]);
    let client = client_for(&sim.base_url());
    client.login("letmein").await.expect("login");
    client.clear_device_list().await.expect("clear");
    assert_eq!(sim.clear_devices_count(), 1);
    let list = client.devices().await.expect("devices after clear");
    assert!(list.is_empty());
    sim.shutdown().await;
}
