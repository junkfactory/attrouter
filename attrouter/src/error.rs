//! Error types for the attrouter client.

use std::io;
use std::net::{AddrParseError, Ipv4Addr};

/// Errors that can occur when talking to an AT&T residential gateway.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid address: {0}")]
    Addr(#[from] AddrParseError),

    #[error("gateway at {0} does not look like an AT&T device")]
    NotAttGateway(Ipv4Addr),

    #[error("could not determine the default IPv4 gateway")]
    GatewayNotFound,

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("not authenticated - call login() first")]
    NotAuthenticated,

    #[error("missing form nonce on page {page}")]
    MissingNonce { page: String },

    #[error("could not parse field `{field}` on page {page}")]
    ParseField { page: String, field: String },

    #[error("unexpected response from gateway: {0}")]
    UnexpectedResponse(String),
}

pub type Result<T> = std::result::Result<T, Error>;
