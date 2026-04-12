//! HTTP client for an AT&T residential gateway.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client as HttpClient;
use reqwest::cookie::Jar;

use crate::discovery;
use crate::error::Result;

const USER_AGENT: &str = concat!("attrouter/", env!("CARGO_PKG_VERSION"));
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// A client for talking to an AT&T residential gateway (BGW210, BGW320,
/// NVG589, 5268AC, etc.) over its web interface.
///
/// The client is cheap to clone — internally it shares a `reqwest::Client`
/// and a cookie jar so authenticated state is preserved.
pub struct Client {
    pub(crate) http: HttpClient,
    pub(crate) base_url: String,
    pub(crate) ip: Ipv4Addr,
    #[allow(dead_code)]
    pub(crate) cookies: Arc<Jar>,
}

impl Client {
    /// Build a client for a gateway at the given IP address. Equivalent to
    /// `Client::with_base_url(format!("http://{ip}"))`.
    pub fn new(ip: Ipv4Addr) -> Result<Self> {
        Self::with_base_url(format!("http://{ip}"))
    }

    /// Build a client pointed at an arbitrary base URL.
    ///
    /// Useful for test setups where the gateway is being impersonated on
    /// `127.0.0.1:<ephemeral port>`, for routing through a local proxy, or
    /// for non-standard gateway configurations.
    ///
    /// The base URL must not include a trailing slash or path component —
    /// the client appends paths like `/cgi-bin/sysinfo.ha` directly.
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();
        let cookies = Arc::new(Jar::default());
        let http = HttpClient::builder()
            .user_agent(USER_AGENT)
            .cookie_provider(cookies.clone())
            .timeout(DEFAULT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        let ip = extract_ipv4_host(&base_url).unwrap_or(Ipv4Addr::LOCALHOST);
        Ok(Self {
            http,
            base_url,
            ip,
            cookies,
        })
    }

    /// Build a client for the system's default IPv4 gateway.
    ///
    /// Reads the OS routing table (Linux only). Returns
    /// [`Error::GatewayNotFound`](crate::Error::GatewayNotFound) if no
    /// default gateway can be determined.
    pub fn for_default_gateway() -> Result<Self> {
        Self::new(discovery::default_gateway()?)
    }

    /// Discover an AT&T gateway on the local network, verify its identity
    /// via [`Client::sysinfo`], and return a client for it.
    pub async fn discover() -> Result<Self> {
        discovery::discover().await
    }

    /// The gateway IP.
    pub fn ip(&self) -> Ipv4Addr {
        self.ip
    }

    /// The base URL of the gateway (`http://<ip>`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub(crate) async fn get(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        tracing::debug!(%url, "GET");
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        tracing::trace!(bytes = text.len(), "GET response");
        Ok(text)
    }

    pub(crate) async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Result<String> {
        let url = self.url(path);
        tracing::debug!(%url, field_count = form.len(), "POST form");
        let resp = self
            .http
            .post(&url)
            .form(form)
            .send()
            .await?
            .error_for_status()?;
        let text = resp.text().await?;
        tracing::trace!(bytes = text.len(), "POST response");
        Ok(text)
    }
}

/// Best-effort IPv4 host extraction from a base URL like
/// `http://192.168.1.254` or `http://127.0.0.1:54321`.
fn extract_ipv4_host(base_url: &str) -> Option<Ipv4Addr> {
    let after_scheme = base_url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(base_url);
    let host_end = after_scheme.find([':', '/']).unwrap_or(after_scheme.len());
    after_scheme[..host_end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_ipv4_host_parses_bare_host() {
        assert_eq!(
            extract_ipv4_host("http://192.168.1.254"),
            Some(Ipv4Addr::new(192, 168, 1, 254))
        );
    }

    #[test]
    fn extract_ipv4_host_parses_host_with_port() {
        assert_eq!(
            extract_ipv4_host("http://127.0.0.1:54321"),
            Some(Ipv4Addr::LOCALHOST)
        );
    }

    #[test]
    fn extract_ipv4_host_parses_host_with_port_and_path() {
        assert_eq!(
            extract_ipv4_host("http://10.0.0.1:80/cgi-bin/sysinfo.ha"),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
    }

    #[test]
    fn extract_ipv4_host_returns_none_for_hostname() {
        assert_eq!(extract_ipv4_host("http://gateway.local"), None);
    }

    #[test]
    fn with_base_url_sets_ip_field() {
        let c = Client::with_base_url("http://127.0.0.1:8080").unwrap();
        assert_eq!(c.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(c.base_url(), "http://127.0.0.1:8080");
    }

    #[test]
    fn new_uses_with_base_url() {
        let c = Client::new(Ipv4Addr::new(192, 168, 1, 254)).unwrap();
        assert_eq!(c.base_url(), "http://192.168.1.254");
        assert_eq!(c.ip(), Ipv4Addr::new(192, 168, 1, 254));
    }
}
