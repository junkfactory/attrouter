//! Login flow for the AT&T gateway.
//!
//! The gateway uses a classic form login protected by a per-page nonce. The
//! client must:
//!
//! 1. `GET /cgi-bin/login.ha` to receive a `SessionID` cookie and a nonce.
//! 2. Compute `hashpassword = md5(access_code + nonce)` as lowercase hex.
//! 3. `POST /cgi-bin/login.ha` with:
//!    - `nonce` (echoed back)
//!    - `password` = `"*"` repeated `access_code.len()` times (what the
//!      router's login.js does — the field acts as a length hint only)
//!    - `hashpassword` = the hex digest from step 2
//!    - `Continue` = `"Continue"` (the submit button name)
//!
//! The device access code is printed on a label on the bottom / side of the
//! gateway.

use md5::{Digest, Md5};
use scraper::Html;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::parse;

pub(crate) const LOGIN_PATH: &str = "/cgi-bin/login.ha";

impl Client {
    /// Authenticate with the gateway's device access code.
    ///
    /// The access code is the 12-character alphanumeric string printed on
    /// the label attached to the side or bottom of the gateway (also called
    /// "Device Access Code" in the web UI).
    ///
    /// On success the session cookie is stored in the client's cookie jar
    /// and subsequent requests will be authenticated.
    pub async fn login(&self, access_code: &str) -> Result<()> {
        let page = self.get(LOGIN_PATH).await?;
        let html = Html::parse_document(&page);
        let nonce = parse::find_nonce(&html, LOGIN_PATH)?;

        let mut hasher = Md5::new();
        hasher.update(access_code.as_bytes());
        hasher.update(nonce.as_bytes());
        let hashpassword = hex::encode(hasher.finalize());

        // The router's login.js replaces each character of the password
        // input with a `*` mask before submitting.
        let masked: String = "*".repeat(access_code.chars().count());

        let form: [(&str, &str); 4] = [
            ("nonce", &nonce),
            ("password", &masked),
            ("hashpassword", &hashpassword),
            ("Continue", "Continue"),
        ];
        let body = self.post_form(LOGIN_PATH, &form).await?;
        let body_html = Html::parse_document(&body);

        // If the gateway rejected the access code it renders the login page
        // again (same <title>Login</title>).
        if parse::is_login_page(&body_html) {
            return Err(Error::AuthFailed(
                "device access code was rejected by the gateway".into(),
            ));
        }
        tracing::info!("login successful");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_hex_md5_of_concatenation() {
        // Mirrors the router's login.js:
        //     hash = hex_md5(password + nonce)
        let mut h = Md5::new();
        h.update(b"abcdef");
        h.update(b"1234");
        let expected = hex::encode(h.finalize());

        let mut h2 = Md5::new();
        h2.update(b"abcdef1234");
        assert_eq!(expected, hex::encode(h2.finalize()));
        assert_eq!(expected.len(), 32);
    }
}
