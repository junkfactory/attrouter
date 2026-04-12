//! High-level status queries against the gateway web UI.
//!
//! None of the queries in this module require authentication — the AT&T
//! gateway exposes its status, broadband, LAN, and device-list pages to any
//! caller on the LAN. Modifying endpoints live in [`crate::control`].

use std::sync::LazyLock;

use scraper::{Html, Selector};

use crate::client::Client;
use crate::error::Result;
use crate::model::{BroadbandStatus, Device, InterfaceStats, LanInterface, LanStatus, SystemInfo};
use crate::parse;

const SYSINFO_PATH: &str = "/cgi-bin/sysinfo.ha";
const BROADBAND_PATH: &str = "/cgi-bin/broadbandstatistics.ha";
const DEVICES_PATH: &str = "/cgi-bin/devices.ha";
const LAN_PATH: &str = "/cgi-bin/lanstatistics.ha";

static DEVICE_TABLE_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("table.table100").unwrap());
static LAN_INTERFACE_TABLE_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("table.table100").unwrap());

impl Client {
    /// Fetch basic information about the gateway device.
    pub async fn sysinfo(&self) -> Result<SystemInfo> {
        let body = self.get(SYSINFO_PATH).await?;
        let html = Html::parse_document(&body);
        let map = parse::th_td_map(&html);
        Ok(SystemInfo {
            manufacturer: map.get("Manufacturer").cloned(),
            model_number: map.get("Model Number").cloned(),
            serial_number: map.get("Serial Number").cloned(),
            software_version: map.get("Software Version").cloned(),
            mac_address: map.get("MAC Address").cloned(),
            first_use_date: map.get("First Use Date").cloned(),
            time_since_last_reboot: map.get("Time Since Last Reboot").cloned(),
            current_date_time: map.get("Current Date/Time").cloned(),
            hardware_version: map.get("Hardware Version").cloned(),
        })
    }

    /// Fetch WAN / broadband status, including interface statistics.
    pub async fn broadband_status(&self) -> Result<BroadbandStatus> {
        let body = self.get(BROADBAND_PATH).await?;
        let html = Html::parse_document(&body);
        let map = parse::th_td_map(&html);

        let status = BroadbandStatus {
            connection_source: map.get("Broadband Connection Source").cloned(),
            connection: map.get("Broadband Connection").cloned(),
            network_type: map.get("Broadband Network Type").cloned(),
            ipv4_address: map.get("Broadband IPv4 Address").cloned(),
            gateway_ipv4_address: map.get("Gateway IPv4 Address").cloned(),
            mac_address: map.get("MAC Address").cloned(),
            primary_dns: map.get("Primary DNS").cloned(),
            secondary_dns: map.get("Secondary DNS").cloned(),
            mtu: map.get("MTU").and_then(|s| s.parse().ok()),
            line_state: map.get("Line State").cloned(),
            current_speed_mbps: map.get("Current Speed (Mbps)").and_then(|s| s.parse().ok()),
            current_duplex: map.get("Current Duplex").cloned(),
            ipv6_status: map.get("Status").cloned(),
            ipv6_service_type: map.get("Service Type").cloned(),
            ipv6_global_address: map.get("Global Unicast IPv6 Address").cloned(),
            ipv6_link_local: map.get("Link Local Address").cloned(),
            ipv6_gateway: map.get("Default IPv6 Gateway Address").cloned(),
            ipv4_stats: parse_interface_stats(&map),
            ipv6_stats: InterfaceStats::default(),
        };
        Ok(status)
    }

    /// Fetch the LAN / home-network summary.
    pub async fn lan_status(&self) -> Result<LanStatus> {
        let body = self.get(LAN_PATH).await?;
        let html = Html::parse_document(&body);
        let map = parse::th_td_map(&html);

        let interfaces = parse_lan_interfaces(&html);

        Ok(LanStatus {
            device_ipv4_address: map.get("Device IPv4 Address").cloned(),
            dhcpv4_netmask: map.get("DHCPv4 Netmask").cloned(),
            dhcp_server: map.get("DHCP Server").cloned(),
            dhcpv4_start: map.get("DHCPv4 Start Address").cloned(),
            dhcpv4_end: map.get("DHCPv4 End Address").cloned(),
            dhcp_leases_available: map
                .get("DHCP Leases Available")
                .and_then(|s| s.parse().ok()),
            dhcp_leases_allocated: map
                .get("DHCP Leases Allocated")
                .and_then(|s| s.parse().ok()),
            dhcp_primary_pool: map.get("DHCP Primary Pool").cloned(),
            secondary_subnet: map.get("Secondary Subnet").cloned(),
            cascaded_router_status: map.get("Cascaded Router Status").cloned(),
            ip_passthrough_status: map.get("IP Passthrough Status").cloned(),
            interfaces,
        })
    }

    /// Fetch the list of LAN devices known to the gateway.
    pub async fn devices(&self) -> Result<Vec<Device>> {
        let body = self.get(DEVICES_PATH).await?;
        let html = Html::parse_document(&body);

        // The devices page has one <table class="table100"> containing every
        // device, with <tr><td colspan="2"><hr class="reshr"/></td></tr>
        // separators between records.
        let Some(table) = html.select(&DEVICE_TABLE_SEL).next() else {
            return Ok(Vec::new());
        };
        let rows_sel = Selector::parse("tr").unwrap();
        let groups = parse::split_at_hr(table.select(&rows_sel));

        let mut devices = Vec::with_capacity(groups.len());
        for group in groups {
            let map = parse::th_td_map_rows(group);
            if map.is_empty() || !map.contains_key("MAC Address") {
                continue;
            }
            devices.push(build_device(&map));
        }
        Ok(devices)
    }
}

fn parse_interface_stats(map: &std::collections::HashMap<String, String>) -> InterfaceStats {
    InterfaceStats {
        receive_packets: map.get("Receive Packets").and_then(|s| s.parse().ok()),
        transmit_packets: map.get("Transmit Packets").and_then(|s| s.parse().ok()),
        receive_bytes: map.get("Receive Bytes").and_then(|s| s.parse().ok()),
        transmit_bytes: map.get("Transmit Bytes").and_then(|s| s.parse().ok()),
        receive_unicast: map.get("Receive Unicast").and_then(|s| s.parse().ok()),
        transmit_unicast: map.get("Transmit Unicast").and_then(|s| s.parse().ok()),
        receive_multicast: map.get("Receive Multicast").and_then(|s| s.parse().ok()),
        transmit_multicast: map.get("Transmit Multicast").and_then(|s| s.parse().ok()),
        receive_drops: map.get("Receive Drops").and_then(|s| s.parse().ok()),
        transmit_drops: map.get("Transmit Drops").and_then(|s| s.parse().ok()),
        receive_errors: map.get("Receive Errors").and_then(|s| s.parse().ok()),
        transmit_errors: map.get("Transmit Errors").and_then(|s| s.parse().ok()),
    }
}

fn parse_lan_interfaces(html: &Html) -> Vec<LanInterface> {
    // The LAN status page has an "Interfaces" section that's the *second*
    // `table.table100` on the page. It's structured as:
    //   <thead><tr><th>Interface</th><th>Status</th>
    //              <th>Active Devices</th><th>Inactive Devices</th></tr></thead>
    //   <tbody><tr><td>Ethernet</td><td>Enabled</td><td>1</td><td>0</td></tr>...
    let tables: Vec<_> = html.select(&LAN_INTERFACE_TABLE_SEL).collect();
    let Some(table) = tables.get(1) else {
        return Vec::new();
    };
    let row_sel = Selector::parse("tbody tr").unwrap();
    let cell_sel = Selector::parse("td").unwrap();
    let mut out = Vec::new();
    for row in table.select(&row_sel) {
        let cells: Vec<String> = row.select(&cell_sel).map(|c| parse::text_of(&c)).collect();
        if cells.len() < 4 {
            continue;
        }
        out.push(LanInterface {
            name: cells[0].clone(),
            status: Some(cells[1].clone()).filter(|s| !s.is_empty()),
            active_devices: cells[2].parse().ok(),
            inactive_devices: cells[3].parse().ok(),
        });
    }
    out
}

fn build_device(map: &std::collections::HashMap<String, Vec<String>>) -> Device {
    let first = |k: &str| map.get(k).and_then(|v| v.first().cloned());
    let mac = first("MAC Address");
    let name = first("Name");

    // Some device blocks have a combined "IPv4 Address / Name" row; others
    // have a standalone "Name" row (for currently-offline devices).
    let (ipv4, combined_name) = match first("IPv4 Address / Name") {
        Some(s) => {
            let mut parts = s.splitn(2, '/');
            let ip = parts
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let n = parts
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            (ip, n)
        }
        None => (None, None),
    };
    let resolved_name = name.or(combined_name);

    let ipv6_addresses: Vec<String> = map.get("IPv6 Address").cloned().unwrap_or_default();

    let mesh_client = first("Mesh Client").map(|s| s.eq_ignore_ascii_case("yes"));

    Device {
        mac_address: mac,
        name: resolved_name,
        ipv4_address: ipv4,
        ipv6_addresses,
        last_activity: first("Last Activity"),
        status: first("Status"),
        allocation: first("Allocation"),
        connection_type: first("Connection Type"),
        connection_speed: first("Connection Speed"),
        mesh_client,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_device_splits_ipv4_and_name() {
        let mut map: std::collections::HashMap<String, Vec<String>> = Default::default();
        map.insert("MAC Address".into(), vec!["04:42:1a:06:d9:67".into()]);
        map.insert(
            "IPv4 Address / Name".into(),
            vec!["192.168.1.70 / islay".into()],
        );
        map.insert("Status".into(), vec!["on".into()]);
        map.insert("Mesh Client".into(), vec!["No".into()]);

        let device = build_device(&map);
        assert_eq!(device.mac_address.as_deref(), Some("04:42:1a:06:d9:67"));
        assert_eq!(device.ipv4_address.as_deref(), Some("192.168.1.70"));
        assert_eq!(device.name.as_deref(), Some("islay"));
        assert_eq!(device.status.as_deref(), Some("on"));
        assert_eq!(device.mesh_client, Some(false));
    }

    #[test]
    fn build_device_collects_ipv6_addresses() {
        let mut map: std::collections::HashMap<String, Vec<String>> = Default::default();
        map.insert("MAC Address".into(), vec!["04:42:1a:06:d9:67".into()]);
        map.insert(
            "IPv6 Address".into(),
            vec![
                "2600:1700:87d0:d4a0::47".into(),
                "fe80::406b:f3e5:45e1:d491".into(),
            ],
        );
        let device = build_device(&map);
        assert_eq!(device.ipv6_addresses.len(), 2);
    }
}
