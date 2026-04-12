//! Discovery of AT&T residential gateways on the local network.
//!
//! On Linux, [`default_gateway`] reads `/proc/net/route` to find the system's
//! default IPv4 gateway. On other platforms it returns
//! [`Error::GatewayNotFound`] — pass an explicit IP instead.
//!
//! [`discover`] combines gateway lookup with a probe against the candidate IP
//! to confirm that an AT&T gateway is actually reachable there.

use std::fs;
use std::net::Ipv4Addr;

use crate::client::Client;
use crate::error::{Error, Result};

/// The traditional default IP for AT&T residential gateways.
pub const DEFAULT_GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 254);

/// Find the system's default IPv4 gateway by reading `/proc/net/route`.
#[cfg(target_os = "linux")]
pub fn default_gateway() -> Result<Ipv4Addr> {
    const RTF_GATEWAY: u32 = 0x2;
    let contents = fs::read_to_string("/proc/net/route")?;
    for line in contents.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let _iface = cols.next();
        let dest = cols.next();
        let gw = cols.next();
        let flags = cols.next();
        let (Some(dest), Some(gw), Some(flags)) = (dest, gw, flags) else {
            continue;
        };
        let flags_n = u32::from_str_radix(flags, 16).unwrap_or(0);
        if dest == "00000000" && (flags_n & RTF_GATEWAY) != 0 {
            let gw_n = u32::from_str_radix(gw, 16).map_err(|_| Error::GatewayNotFound)?;
            return Ok(Ipv4Addr::from(gw_n.to_le_bytes()));
        }
    }
    Err(Error::GatewayNotFound)
}

#[cfg(not(target_os = "linux"))]
pub fn default_gateway() -> Result<Ipv4Addr> {
    Err(Error::GatewayNotFound)
}

/// Candidate IPs to try when the OS routing table is unavailable.
const FALLBACK_CANDIDATES: &[Ipv4Addr] = &[
    Ipv4Addr::new(192, 168, 1, 254),
    Ipv4Addr::new(192, 168, 0, 254),
    Ipv4Addr::new(172, 27, 0, 1),
];

/// Discover an AT&T gateway on the local network.
///
/// Tries the OS default gateway first, then a short list of known default
/// IPs. For each candidate, probes [`Client::sysinfo`] and returns the first
/// client that identifies as an AT&T device.
pub async fn discover() -> Result<Client> {
    let mut candidates: Vec<Ipv4Addr> = Vec::new();
    if let Ok(gw) = default_gateway() {
        candidates.push(gw);
    }
    for ip in FALLBACK_CANDIDATES {
        if !candidates.contains(ip) {
            candidates.push(*ip);
        }
    }

    let mut last_err: Option<Error> = None;
    for ip in candidates {
        let client = Client::new(ip)?;
        match client.sysinfo().await {
            Ok(info) => {
                let looks_att = info
                    .manufacturer
                    .as_deref()
                    .map(looks_like_att_vendor)
                    .unwrap_or(false)
                    || info
                        .model_number
                        .as_deref()
                        .map(looks_like_att_model)
                        .unwrap_or(false);
                if looks_att {
                    return Ok(client);
                }
                last_err = Some(Error::NotAttGateway(ip));
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or(Error::GatewayNotFound))
}

fn looks_like_att_vendor(vendor: &str) -> bool {
    let v = vendor.to_ascii_uppercase();
    matches!(
        v.as_str(),
        "HUMAX" | "ARRIS" | "PACE" | "NOKIA" | "NVG" | "MOTOROLA"
    ) || v.contains("HUMAX")
        || v.contains("ARRIS")
        || v.contains("PACE")
        || v.contains("NOKIA")
}

fn looks_like_att_model(model: &str) -> bool {
    let m = model.to_ascii_uppercase();
    m.starts_with("BGW")
        || m.starts_with("NVG")
        || m.starts_with("5268")
        || m.starts_with("589")
        || m.starts_with("599")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_att_vendor_recognizes_known_vendors() {
        for v in ["HUMAX", "humax", "Arris", "PACE", "Nokia", "MOTOROLA"] {
            assert!(looks_like_att_vendor(v), "{v} should match");
        }
    }

    #[test]
    fn looks_like_att_vendor_rejects_others() {
        for v in ["Netgear", "Asus", "TP-Link", "Ubiquiti", ""] {
            assert!(!looks_like_att_vendor(v), "{v} should not match");
        }
    }

    #[test]
    fn looks_like_att_model_matches_bgw_nvg_5268() {
        for m in [
            "BGW210-700",
            "BGW320-500",
            "BGW320-505",
            "NVG589",
            "NVG599",
            "5268AC",
        ] {
            assert!(looks_like_att_model(m), "{m} should match");
        }
    }

    #[test]
    fn looks_like_att_model_rejects_random_models() {
        for m in ["RT-AX88U", "RAX120", "", "GatewayPro"] {
            assert!(!looks_like_att_model(m), "{m} should not match");
        }
    }
}
