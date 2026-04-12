//! HTML scraping helpers for the AT&T gateway pages.
//!
//! The AT&T gateway web UI renders its data as plain XHTML tables. Every
//! status page follows the same `<tr><th>label</th><td>value</td></tr>`
//! pattern, and every form page embeds a CSRF nonce as a hidden
//! `<input name="nonce">` field. This module has small helpers for both.

use std::collections::HashMap;
use std::sync::LazyLock;

use scraper::{ElementRef, Html, Selector};

use crate::error::{Error, Result};

static TR_SEL: LazyLock<Selector> = LazyLock::new(|| Selector::parse("tr").unwrap());
static TH_SEL: LazyLock<Selector> = LazyLock::new(|| Selector::parse("th").unwrap());
static TD_SEL: LazyLock<Selector> = LazyLock::new(|| Selector::parse("td").unwrap());
static NONCE_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"input[name="nonce"]"#).unwrap());
static HR_SEL: LazyLock<Selector> = LazyLock::new(|| Selector::parse("hr.reshr").unwrap());

/// Collapse an element's inner text into a single whitespace-normalised
/// string. Text fragments separated by `<br>` or other block-level elements
/// are joined with a single space so adjacent labels don't glue together.
pub(crate) fn text_of(el: &ElementRef<'_>) -> String {
    el.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collect every `<tr><th>label</th><td>value</td></tr>` in the document
/// into a `label -> value` map. Later duplicates of a label are ignored,
/// which matches the "first occurrence wins" behaviour of the existing
/// AT&T pages.
pub(crate) fn th_td_map(html: &Html) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for row in html.select(&TR_SEL) {
        let Some(th) = row.select(&TH_SEL).next() else {
            continue;
        };
        let Some(td) = row.select(&TD_SEL).next() else {
            continue;
        };
        let label = text_of(&th);
        let value = text_of(&td);
        if !label.is_empty() {
            out.entry(label).or_insert(value);
        }
    }
    out
}

/// Like [`th_td_map`] but scoped to rows collected from an iterator — used
/// for splitting a device list into per-device sections.
pub(crate) fn th_td_map_rows<'a>(
    rows: impl IntoIterator<Item = ElementRef<'a>>,
) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let Some(th) = row.select(&TH_SEL).next() else {
            continue;
        };
        let Some(td) = row.select(&TD_SEL).next() else {
            continue;
        };
        let label = text_of(&th);
        let value = text_of(&td);
        if !label.is_empty() {
            out.entry(label).or_default().push(value);
        }
    }
    out
}

/// Find the CSRF nonce on a gateway form page.
pub(crate) fn find_nonce(html: &Html, page: &str) -> Result<String> {
    html.select(&NONCE_SEL)
        .next()
        .and_then(|el| el.value().attr("value"))
        .map(|s| s.to_string())
        .ok_or_else(|| Error::MissingNonce {
            page: page.to_string(),
        })
}

/// True if the rendered page is the login form (i.e. the gateway kicked us
/// back because we lacked a session).
pub(crate) fn is_login_page(html: &Html) -> bool {
    static TITLE_SEL: LazyLock<Selector> = LazyLock::new(|| Selector::parse("title").unwrap());
    html.select(&TITLE_SEL)
        .next()
        .map(|el| {
            el.text()
                .collect::<String>()
                .trim()
                .eq_ignore_ascii_case("Login")
        })
        .unwrap_or(false)
}

/// Split a `<tr>` iterator into groups at rows containing an `<hr class="reshr">`.
///
/// The devices page renders all devices in one big `<table class="table100">`
/// and separates records with a `<tr><td colspan="2"><hr class="reshr"/></td></tr>`
/// spacer row. This returns one `Vec<ElementRef>` per device.
pub(crate) fn split_at_hr<'a>(
    rows: impl Iterator<Item = ElementRef<'a>>,
) -> Vec<Vec<ElementRef<'a>>> {
    let mut groups: Vec<Vec<ElementRef<'a>>> = Vec::new();
    let mut current: Vec<ElementRef<'a>> = Vec::new();
    for row in rows {
        if row.select(&HR_SEL).next().is_some() {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            current.push(row);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSINFO_FIXTURE: &str = include_str!("testdata/sysinfo.html");
    const BROADBAND_FIXTURE: &str = include_str!("testdata/broadbandstatistics.html");
    const DEVICES_FIXTURE: &str = include_str!("testdata/devices.html");
    const LAN_FIXTURE: &str = include_str!("testdata/lanstatistics.html");

    #[test]
    fn th_td_map_extracts_label_value_pairs() {
        let html = Html::parse_document(
            r#"<table>
                <tr><th>Model Number</th><td>BGW320-500</td></tr>
                <tr><th>Software Version</th><td>6.34.7</td></tr>
            </table>"#,
        );
        let map = th_td_map(&html);
        assert_eq!(
            map.get("Model Number").map(String::as_str),
            Some("BGW320-500")
        );
        assert_eq!(
            map.get("Software Version").map(String::as_str),
            Some("6.34.7")
        );
    }

    #[test]
    fn find_nonce_reads_hidden_input() {
        let html = Html::parse_document(
            r#"<form><input type="hidden" name="nonce" value="abc123"/></form>"#,
        );
        assert_eq!(find_nonce(&html, "/test").unwrap(), "abc123");
    }

    #[test]
    fn find_nonce_returns_missing_nonce_when_absent() {
        let html = Html::parse_document("<form></form>");
        let err = find_nonce(&html, "/cgi-bin/x.ha").unwrap_err();
        assert!(matches!(err, Error::MissingNonce { .. }));
    }

    #[test]
    fn is_login_page_matches_title() {
        let html = Html::parse_document("<html><head><title>Login</title></head></html>");
        assert!(is_login_page(&html));
        let html =
            Html::parse_document("<html><head><title>System Information</title></head></html>");
        assert!(!is_login_page(&html));
    }

    #[test]
    fn split_at_hr_produces_one_group_per_device() {
        let html = Html::parse_document(
            r#"<table>
                <tr><th>MAC</th><td>AA</td></tr>
                <tr><th>Name</th><td>alpha</td></tr>
                <tr><td colspan="2"><hr class="reshr"/></td></tr>
                <tr><th>MAC</th><td>BB</td></tr>
                <tr><th>Name</th><td>beta</td></tr>
                <tr><td colspan="2"><hr class="reshr"/></td></tr>
                <tr><th>MAC</th><td>CC</td></tr>
            </table>"#,
        );
        let rows: Vec<_> = html.select(&TR_SEL).collect();
        let groups = split_at_hr(rows.into_iter());
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1].len(), 2);
        assert_eq!(groups[2].len(), 1);
    }

    #[test]
    fn text_of_joins_fragments_with_spaces() {
        let html = Html::parse_document(
            r#"<html><body><div id="t">Wi-Fi<br/>5 GHz Radio-2<br/>Type: Home</div></body></html>"#,
        );
        let sel = Selector::parse("div#t").unwrap();
        let el = html.select(&sel).next().unwrap();
        assert_eq!(text_of(&el), "Wi-Fi 5 GHz Radio-2 Type: Home");
    }

    #[test]
    fn parses_sysinfo_fixture_fields() {
        let html = Html::parse_document(SYSINFO_FIXTURE);
        let map = th_td_map(&html);
        assert_eq!(map.get("Manufacturer").map(String::as_str), Some("HUMAX"));
        assert_eq!(
            map.get("Model Number").map(String::as_str),
            Some("BGW320-500")
        );
        assert_eq!(
            map.get("Serial Number").map(String::as_str),
            Some("TEST1234567890")
        );
        assert_eq!(
            map.get("MAC Address").map(String::as_str),
            Some("00:11:22:33:44:55")
        );
    }

    #[test]
    fn parses_broadband_fixture_ipv4_and_stats() {
        let html = Html::parse_document(BROADBAND_FIXTURE);
        let map = th_td_map(&html);
        assert_eq!(
            map.get("Broadband IPv4 Address").map(String::as_str),
            Some("192.0.2.1")
        );
        assert_eq!(
            map.get("Gateway IPv4 Address").map(String::as_str),
            Some("192.0.2.254")
        );
        assert_eq!(map.get("MTU").map(String::as_str), Some("1500"));
        assert_eq!(
            map.get("Current Speed (Mbps)").map(String::as_str),
            Some("10000")
        );
        // stats counters are present
        assert!(map.contains_key("Receive Packets"));
        assert!(map.contains_key("Transmit Bytes"));
    }

    #[test]
    fn parses_lan_fixture_dhcp_range() {
        let html = Html::parse_document(LAN_FIXTURE);
        let map = th_td_map(&html);
        assert_eq!(
            map.get("Device IPv4 Address").map(String::as_str),
            Some("192.168.1.254")
        );
        assert_eq!(
            map.get("DHCPv4 Netmask").map(String::as_str),
            Some("255.255.255.0")
        );
        assert_eq!(map.get("DHCP Server").map(String::as_str), Some("On"));
        assert_eq!(
            map.get("DHCPv4 Start Address").map(String::as_str),
            Some("192.168.1.64")
        );
    }

    #[test]
    fn splits_devices_fixture_into_records() {
        let html = Html::parse_document(DEVICES_FIXTURE);
        let table_sel = Selector::parse("table.table100").unwrap();
        let table = html.select(&table_sel).next().unwrap();
        let rows: Vec<_> = table.select(&TR_SEL).collect();
        let groups = split_at_hr(rows.into_iter());
        // Our sanitized capture has 9 devices.
        assert_eq!(
            groups.len(),
            9,
            "expected 9 device groups, got {}",
            groups.len()
        );
        for g in &groups {
            let map = th_td_map_rows(g.iter().copied());
            assert!(
                map.contains_key("MAC Address"),
                "every device group should contain a MAC Address row"
            );
        }
    }
}
