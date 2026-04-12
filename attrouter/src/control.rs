//! Mutating actions against the gateway.
//!
//! Every endpoint in this module requires a prior [`Client::login`] call.
//! Each action first fetches the containing page to pick up a fresh CSRF
//! nonce, then POSTs the appropriate form fields.

use scraper::Html;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::parse;

const HOME_PATH: &str = "/cgi-bin/home.ha";
const RESTART_PATH: &str = "/cgi-bin/restart.ha";
const CRESTART_PATH: &str = "/cgi-bin/crestart.ha?1";
const WRESTART_2G_PATH: &str = "/cgi-bin/wrestart.ha?1";
const WRESTART_5G_PATH: &str = "/cgi-bin/wrestart.ha?2";

/// Which Wi-Fi radio to target for a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiBand {
    /// 2.4 GHz radio.
    Ghz24,
    /// 5 GHz radio.
    Ghz5,
}

impl Client {
    /// Reboot the gateway.
    ///
    /// Requires prior [`Client::login`]. The router immediately tears down
    /// the HTTP connection and begins rebooting — the return value here
    /// only means the request was accepted.
    pub async fn restart_device(&self) -> Result<()> {
        let (nonce, submit_name) = self
            .nonce_and_submit(RESTART_PATH, &["Restart", "DevRestart", "Continue"])
            .await?;
        let form: [(&str, &str); 2] = [("nonce", &nonce), (submit_name, "Restart")];
        // A successful restart request will often drop the TCP connection
        // mid-response — treat connection-reset as success.
        match self.post_form(RESTART_PATH, &form).await {
            Ok(_) => Ok(()),
            Err(Error::Http(e)) if is_connection_dropped(&e) => {
                tracing::debug!("gateway dropped connection after restart request");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Restart the broadband (WAN) connection.
    ///
    /// This is the "Restart" button next to the Broadband status on the
    /// main Device page. Does not reboot the gateway, but does drop and
    /// re-establish the upstream link.
    pub async fn restart_broadband(&self) -> Result<()> {
        let home = self.get(HOME_PATH).await?;
        let home_html = Html::parse_document(&home);
        if parse::is_login_page(&home_html) {
            return Err(Error::NotAuthenticated);
        }
        let nonce = parse::find_nonce(&home_html, HOME_PATH)?;
        let form: [(&str, &str); 2] = [("nonce", &nonce), ("Broadband", "Restart")];
        self.post_form(CRESTART_PATH, &form).await?;
        Ok(())
    }

    /// Restart a Wi-Fi radio.
    pub async fn restart_wifi(&self, band: WifiBand) -> Result<()> {
        let (path, submit) = match band {
            WifiBand::Ghz24 => (WRESTART_2G_PATH, "WRestart1"),
            WifiBand::Ghz5 => (WRESTART_5G_PATH, "WRestart2"),
        };
        let home = self.get(HOME_PATH).await?;
        let home_html = Html::parse_document(&home);
        if parse::is_login_page(&home_html) {
            return Err(Error::NotAuthenticated);
        }
        let nonce = parse::find_nonce(&home_html, HOME_PATH)?;
        let form: [(&str, &str); 2] = [("nonce", &nonce), (submit, "Restart")];
        self.post_form(path, &form).await?;
        Ok(())
    }

    /// Clear the "rescan devices" list (the "Clear and Rescan for Devices"
    /// button on the Device List page).
    pub async fn clear_device_list(&self) -> Result<()> {
        const DEVICES_PATH: &str = "/cgi-bin/devices.ha";
        let page = self.get(DEVICES_PATH).await?;
        let html = Html::parse_document(&page);
        if parse::is_login_page(&html) {
            return Err(Error::NotAuthenticated);
        }
        let nonce = parse::find_nonce(&html, DEVICES_PATH)?;
        let form: [(&str, &str); 2] =
            [("nonce", &nonce), ("Clear", "Clear and Rescan for Devices")];
        self.post_form(DEVICES_PATH, &form).await?;
        Ok(())
    }

    /// Fetch `path`, verify we're authenticated, and return the page's CSRF
    /// nonce along with whichever of the candidate submit-button names is
    /// actually present on the page.
    async fn nonce_and_submit<'a>(
        &self,
        path: &str,
        candidates: &'a [&'a str],
    ) -> Result<(String, &'a str)> {
        let body = self.get(path).await?;
        let html = Html::parse_document(&body);
        if parse::is_login_page(&html) {
            return Err(Error::NotAuthenticated);
        }
        let nonce = parse::find_nonce(&html, path)?;
        let submit = candidates
            .iter()
            .copied()
            .find(|name| body.contains(&format!(r#"name="{name}""#)))
            .unwrap_or(candidates[0]);
        Ok((nonce, submit))
    }
}

fn is_connection_dropped(e: &reqwest::Error) -> bool {
    e.is_connect() || e.is_request() || e.is_timeout() || e.is_body() || e.is_decode()
}
