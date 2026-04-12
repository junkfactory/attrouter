//! `attrctl` — command-line front end for the attrouter library.

use std::io::{self, IsTerminal, Write};
use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use attrouter::{Client, WifiBand};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "attrctl",
    version,
    about = "Query and control AT&T residential gateway routers"
)]
struct Cli {
    /// Gateway IP address. Defaults to auto-discovery.
    #[arg(long, global = true, env = "ATTROUTER_IP")]
    ip: Option<Ipv4Addr>,

    /// Device access code for authenticated operations (printed on the
    /// label on the bottom / side of the gateway).
    #[arg(long, global = true, env = "ATTROUTER_ACCESS_CODE")]
    access_code: Option<String>,

    /// Output format for the `show` / `devices` commands.
    #[arg(long, short = 'o', global = true, default_value_t = OutputFormat::Text)]
    output: OutputFormat,

    /// Enable logging at the given level (trace, debug, info, warn, error).
    #[arg(long, global = true, env = "LOG_LEVEL")]
    log_level: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    Yaml,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
            OutputFormat::Yaml => "yaml",
        };
        f.write_str(s)
    }
}

#[derive(Subcommand)]
enum Command {
    /// Discover the gateway and print its IP address.
    Discover,
    /// Show basic device information (model, firmware, MAC, uptime).
    Info,
    /// Show broadband (WAN) status and statistics.
    Broadband,
    /// Show LAN / home-network status.
    Lan,
    /// List LAN devices known to the gateway.
    Devices {
        /// Only show devices whose `status` is `on`.
        #[arg(long)]
        online: bool,
    },
    /// Show everything at once (info + broadband + LAN + devices).
    Status,
    /// Verify the device access code by performing a login.
    Login,
    /// Reboot the gateway. Requires `--access-code`.
    Reboot,
    /// Restart the broadband (WAN) connection. Requires `--access-code`.
    RestartBroadband,
    /// Restart a Wi-Fi radio. Requires `--access-code`.
    RestartWifi {
        /// Which radio to restart.
        #[arg(value_enum)]
        band: WifiBandArg,
    },
    /// Clear the device list. Requires `--access-code`.
    ClearDevices,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum WifiBandArg {
    #[value(name = "2.4")]
    Ghz24,
    #[value(name = "5")]
    Ghz5,
}

impl From<WifiBandArg> for WifiBand {
    fn from(v: WifiBandArg) -> Self {
        match v {
            WifiBandArg::Ghz24 => WifiBand::Ghz24,
            WifiBandArg::Ghz5 => WifiBand::Ghz5,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(level) = cli.log_level.as_deref() {
        tracing_subscriber::fmt()
            .with_env_filter(level)
            .with_writer(io::stderr)
            .init();
    }

    let client = build_client(&cli).await?;

    match cli.command {
        Command::Discover => {
            println!("{}", client.ip());
            let info = client.sysinfo().await?;
            if let Some(model) = &info.model_number {
                println!("model: {model}");
            }
            if let Some(fw) = &info.software_version {
                println!("firmware: {fw}");
            }
        }
        Command::Info => render(&cli, &client.sysinfo().await?)?,
        Command::Broadband => render(&cli, &client.broadband_status().await?)?,
        Command::Lan => render(&cli, &client.lan_status().await?)?,
        Command::Devices { online } => {
            let mut devices = client.devices().await?;
            if online {
                devices.retain(|d| d.status.as_deref() == Some("on"));
            }
            render(&cli, &devices)?;
        }
        Command::Status => {
            let sysinfo = client.sysinfo().await?;
            let broadband = client.broadband_status().await?;
            let lan = client.lan_status().await?;
            let devices = client.devices().await?;
            let summary = Summary {
                gateway: client.ip().to_string(),
                sysinfo,
                broadband,
                lan,
                devices,
            };
            render(&cli, &summary)?;
        }
        Command::Login => {
            let code = cli
                .access_code
                .as_deref()
                .context("--access-code is required for login")?;
            client.login(code).await?;
            println!("login ok");
        }
        Command::Reboot => {
            login_if_needed(&cli, &client).await?;
            if !confirm("reboot the gateway?")? {
                return Ok(());
            }
            client.restart_device().await?;
            println!("reboot requested");
        }
        Command::RestartBroadband => {
            login_if_needed(&cli, &client).await?;
            client.restart_broadband().await?;
            println!("broadband restart requested");
        }
        Command::RestartWifi { band } => {
            login_if_needed(&cli, &client).await?;
            client.restart_wifi(band.into()).await?;
            println!("wi-fi {band:?} restart requested");
        }
        Command::ClearDevices => {
            login_if_needed(&cli, &client).await?;
            client.clear_device_list().await?;
            println!("device list cleared");
        }
    }

    Ok(())
}

async fn build_client(cli: &Cli) -> Result<Client> {
    match cli.ip {
        Some(ip) => Client::new(ip).context("building client"),
        None => Client::discover()
            .await
            .context("discovering AT&T gateway on the local network"),
    }
}

async fn login_if_needed(cli: &Cli, client: &Client) -> Result<()> {
    let code = cli
        .access_code
        .as_deref()
        .context("--access-code is required for this operation")?;
    client.login(code).await?;
    Ok(())
}

#[derive(serde::Serialize)]
struct Summary {
    gateway: String,
    sysinfo: attrouter::SystemInfo,
    broadband: attrouter::BroadbandStatus,
    lan: attrouter::LanStatus,
    devices: Vec<attrouter::Device>,
}

fn render<T: serde::Serialize + RenderText>(cli: &Cli, value: &T) -> Result<()> {
    let mut out = io::stdout().lock();
    match cli.output {
        OutputFormat::Text => value.render_text(&mut out)?,
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut out, value)?;
            writeln!(&mut out)?;
        }
        OutputFormat::Yaml => {
            serde_yaml::to_writer(&mut out, value)?;
        }
    }
    Ok(())
}

trait RenderText {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()>;
}

impl RenderText for attrouter::SystemInfo {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()> {
        write_field(w, "Manufacturer", self.manufacturer.as_deref())?;
        write_field(w, "Model Number", self.model_number.as_deref())?;
        write_field(w, "Serial Number", self.serial_number.as_deref())?;
        write_field(w, "Software Version", self.software_version.as_deref())?;
        write_field(w, "Hardware Version", self.hardware_version.as_deref())?;
        write_field(w, "MAC Address", self.mac_address.as_deref())?;
        write_field(w, "First Use Date", self.first_use_date.as_deref())?;
        write_field(
            w,
            "Time Since Last Reboot",
            self.time_since_last_reboot.as_deref(),
        )?;
        write_field(w, "Current Date/Time", self.current_date_time.as_deref())?;
        Ok(())
    }
}

impl RenderText for attrouter::BroadbandStatus {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()> {
        write_field(w, "Connection", self.connection.as_deref())?;
        write_field(w, "Source", self.connection_source.as_deref())?;
        write_field(w, "Network Type", self.network_type.as_deref())?;
        write_field(w, "IPv4 Address", self.ipv4_address.as_deref())?;
        write_field(w, "IPv4 Gateway", self.gateway_ipv4_address.as_deref())?;
        write_field(w, "Primary DNS", self.primary_dns.as_deref())?;
        write_field(w, "Secondary DNS", self.secondary_dns.as_deref())?;
        write_field_opt_num(w, "MTU", self.mtu)?;
        write_field(w, "Line State", self.line_state.as_deref())?;
        write_field_opt_num(w, "Speed (Mbps)", self.current_speed_mbps)?;
        write_field(w, "Duplex", self.current_duplex.as_deref())?;
        write_field(w, "IPv6 Address", self.ipv6_global_address.as_deref())?;
        write_field(w, "IPv6 Gateway", self.ipv6_gateway.as_deref())?;
        if let Some(rx) = self.ipv4_stats.receive_bytes {
            let tx = self.ipv4_stats.transmit_bytes.unwrap_or(0);
            writeln!(w, "{:<24}{} rx / {} tx bytes", "Traffic", rx, tx)?;
        }
        Ok(())
    }
}

impl RenderText for attrouter::LanStatus {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()> {
        write_field(w, "Device IPv4", self.device_ipv4_address.as_deref())?;
        write_field(w, "DHCPv4 Netmask", self.dhcpv4_netmask.as_deref())?;
        write_field(w, "DHCP Server", self.dhcp_server.as_deref())?;
        let range = match (&self.dhcpv4_start, &self.dhcpv4_end) {
            (Some(a), Some(b)) => Some(format!("{a} - {b}")),
            _ => None,
        };
        write_field(w, "DHCPv4 Range", range.as_deref())?;
        write_field_opt_num(w, "Leases Available", self.dhcp_leases_available)?;
        write_field_opt_num(w, "Leases Allocated", self.dhcp_leases_allocated)?;
        write_field(w, "IP Passthrough", self.ip_passthrough_status.as_deref())?;
        if !self.interfaces.is_empty() {
            writeln!(w, "\nInterfaces:")?;
            for iface in &self.interfaces {
                writeln!(
                    w,
                    "  {:<20}{:<10}{} active, {} inactive",
                    iface.name,
                    iface.status.as_deref().unwrap_or("-"),
                    iface.active_devices.unwrap_or(0),
                    iface.inactive_devices.unwrap_or(0)
                )?;
            }
        }
        Ok(())
    }
}

impl RenderText for Vec<attrouter::Device> {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()> {
        if self.is_empty() {
            writeln!(w, "(no devices)")?;
            return Ok(());
        }
        writeln!(
            w,
            "{:<18}{:<16}{:<20}{:<6}Connection",
            "MAC", "IPv4", "Name", "State"
        )?;
        for d in self {
            writeln!(
                w,
                "{:<18}{:<16}{:<20}{:<6}{}",
                d.mac_address.as_deref().unwrap_or("-"),
                d.ipv4_address.as_deref().unwrap_or("-"),
                truncate(d.name.as_deref().unwrap_or("-"), 20),
                d.status.as_deref().unwrap_or("-"),
                d.connection_type.as_deref().unwrap_or("-"),
            )?;
        }
        Ok(())
    }
}

impl RenderText for Summary {
    fn render_text(&self, w: &mut dyn Write) -> io::Result<()> {
        writeln!(w, "== Gateway {} ==", self.gateway)?;
        self.sysinfo.render_text(w)?;
        writeln!(w, "\n== Broadband ==")?;
        self.broadband.render_text(w)?;
        writeln!(w, "\n== LAN ==")?;
        self.lan.render_text(w)?;
        writeln!(w, "\n== Devices ({}) ==", self.devices.len())?;
        self.devices.render_text(w)?;
        Ok(())
    }
}

fn write_field(w: &mut dyn Write, label: &str, value: Option<&str>) -> io::Result<()> {
    if let Some(v) = value.filter(|s| !s.is_empty()) {
        writeln!(w, "{label:<24}{v}")?;
    }
    Ok(())
}

fn write_field_opt_num<N: std::fmt::Display>(
    w: &mut dyn Write,
    label: &str,
    value: Option<N>,
) -> io::Result<()> {
    if let Some(v) = value {
        writeln!(w, "{label:<24}{v}")?;
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(true);
    }
    eprint!("{prompt} [y/N] ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}
