#![deny(rust_2018_idioms)]
//! Async Rust client for AT&T residential gateway routers.
//!
//! `attrouter` talks to the web UI of AT&T "gateway" routers — the
//! BGW210-700, BGW320-500, 5268AC, NVG589, NVG599 and similar devices —
//! over plain HTTP on the LAN. It supports:
//!
//! - [`Client::discover`] — find the gateway on the local network and
//!   verify it's an AT&T device.
//! - [`Client::sysinfo`], [`Client::broadband_status`],
//!   [`Client::lan_status`], [`Client::devices`] — read-only status
//!   queries, no authentication required.
//! - [`Client::login`] — authenticate with the 12-character device access
//!   code printed on the gateway's label.
//! - [`Client::restart_device`], [`Client::restart_broadband`],
//!   [`Client::restart_wifi`], [`Client::clear_device_list`] — mutating
//!   actions that require a prior [`Client::login`].
//!
//! # Quick start
//!
//! ```rust,no_run
//! use attrouter::Client;
//!
//! # async fn example() -> attrouter::Result<()> {
//! // Auto-detect the gateway on the local network
//! let client = Client::discover().await?;
//! println!("found gateway at {}", client.ip());
//!
//! // Read-only queries don't need authentication
//! let info = client.sysinfo().await?;
//! println!("model {:?}, firmware {:?}", info.model_number, info.software_version);
//!
//! for device in client.devices().await? {
//!     println!("{:?} {:?}", device.mac_address, device.name);
//! }
//!
//! // Mutating actions require the device access code
//! client.login("ABCD1234EFGH").await?;
//! client.restart_broadband().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Supported models
//!
//! Primary development target is the Humax BGW320-500 running firmware
//! `6.34.x`. The library should work against any gateway that serves the
//! classic AT&T `cgi-bin/*.ha` web UI (BGW210, BGW320 variants, older Pace
//! / Arris / 2Wire gateways). Models with a drastically different UI may
//! need additional parsers.

pub mod auth;
pub mod client;
pub mod control;
pub mod discovery;
pub mod error;
pub mod model;
pub(crate) mod parse;
pub mod query;

pub use client::Client;
pub use control::WifiBand;
pub use discovery::{DEFAULT_GATEWAY, default_gateway, discover};
pub use error::{Error, Result};
pub use model::{BroadbandStatus, Device, InterfaceStats, LanInterface, LanStatus, SystemInfo};
