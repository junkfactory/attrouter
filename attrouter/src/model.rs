//! Data types returned by the gateway.

use serde::{Deserialize, Serialize};

/// Information about the gateway device itself.
///
/// Fetched from `/cgi-bin/sysinfo.ha`. Does not require authentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemInfo {
    pub manufacturer: Option<String>,
    pub model_number: Option<String>,
    pub serial_number: Option<String>,
    pub software_version: Option<String>,
    pub mac_address: Option<String>,
    pub first_use_date: Option<String>,
    pub time_since_last_reboot: Option<String>,
    pub current_date_time: Option<String>,
    pub hardware_version: Option<String>,
}

/// Broadband (WAN) status and statistics.
///
/// Fetched from `/cgi-bin/broadbandstatistics.ha`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BroadbandStatus {
    pub connection_source: Option<String>,
    pub connection: Option<String>,
    pub network_type: Option<String>,
    pub ipv4_address: Option<String>,
    pub gateway_ipv4_address: Option<String>,
    pub mac_address: Option<String>,
    pub primary_dns: Option<String>,
    pub secondary_dns: Option<String>,
    pub mtu: Option<u32>,
    pub line_state: Option<String>,
    pub current_speed_mbps: Option<u32>,
    pub current_duplex: Option<String>,
    pub ipv6_status: Option<String>,
    pub ipv6_service_type: Option<String>,
    pub ipv6_global_address: Option<String>,
    pub ipv6_link_local: Option<String>,
    pub ipv6_gateway: Option<String>,
    pub ipv4_stats: InterfaceStats,
    pub ipv6_stats: InterfaceStats,
}

/// Packet / byte counters for a WAN interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterfaceStats {
    pub receive_packets: Option<u64>,
    pub transmit_packets: Option<u64>,
    pub receive_bytes: Option<u64>,
    pub transmit_bytes: Option<u64>,
    pub receive_unicast: Option<u64>,
    pub transmit_unicast: Option<u64>,
    pub receive_multicast: Option<u64>,
    pub transmit_multicast: Option<u64>,
    pub receive_drops: Option<u64>,
    pub transmit_drops: Option<u64>,
    pub receive_errors: Option<u64>,
    pub transmit_errors: Option<u64>,
}

/// LAN / home-network summary.
///
/// Fetched from `/cgi-bin/lanstatistics.ha`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LanStatus {
    pub device_ipv4_address: Option<String>,
    pub dhcpv4_netmask: Option<String>,
    pub dhcp_server: Option<String>,
    pub dhcpv4_start: Option<String>,
    pub dhcpv4_end: Option<String>,
    pub dhcp_leases_available: Option<u32>,
    pub dhcp_leases_allocated: Option<u32>,
    pub dhcp_primary_pool: Option<String>,
    pub secondary_subnet: Option<String>,
    pub cascaded_router_status: Option<String>,
    pub ip_passthrough_status: Option<String>,
    pub interfaces: Vec<LanInterface>,
}

/// One row of the LAN "Interfaces" table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LanInterface {
    pub name: String,
    pub status: Option<String>,
    pub active_devices: Option<u32>,
    pub inactive_devices: Option<u32>,
}

/// A device known to the gateway.
///
/// Fetched from `/cgi-bin/devices.ha`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Device {
    pub mac_address: Option<String>,
    pub name: Option<String>,
    pub ipv4_address: Option<String>,
    pub ipv6_addresses: Vec<String>,
    pub last_activity: Option<String>,
    pub status: Option<String>,
    pub allocation: Option<String>,
    pub connection_type: Option<String>,
    pub connection_speed: Option<String>,
    pub mesh_client: Option<bool>,
}
