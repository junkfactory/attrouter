# attrouter

Async Rust client and CLI for AT&T residential gateway routers — the BGW210,
BGW320, 5268AC, NVG589/599 and similar devices that ship with fiber and
fixed-wireless AT&T service. Talks to the gateway's web UI over plain HTTP
on the LAN; no cloud API involvement.

Primary development target: **Humax BGW320-500 running firmware 6.34.x**.
Should work on most gateways that render the classic `/cgi-bin/*.ha` web
UI. If you hit a model that doesn't parse, patches welcome.

## What it does

Read-only (no authentication required):

- Discover the gateway on the LAN
- System information (model, firmware, MAC, uptime)
- Broadband (WAN) status, IP addresses, traffic counters
- LAN / DHCP status and interface summary
- Full list of devices known to the gateway (wired and Wi-Fi)

Mutating (requires the 12-character device access code printed on the
label under the router):

- Reboot the gateway
- Restart the broadband connection
- Restart a Wi-Fi radio (2.4 GHz or 5 GHz)
- Clear and rescan the device list

## Building and testing

```sh
make help                      # every target with a one-line description
make build                     # cargo build --workspace
make test                      # fmt-check + clippy + unit + simulator
make test-live LIVE_ROUTER=192.168.1.254
```

## Library

```rust
use attrouter::Client;

#[tokio::main]
async fn main() -> attrouter::Result<()> {
    // Auto-detect the gateway on the local network
    let client = Client::discover().await?;
    println!("gateway at {}", client.ip());

    let info = client.sysinfo().await?;
    println!("{:?} firmware {:?}", info.model_number, info.software_version);

    for d in client.devices().await? {
        println!("{:?} {:?}", d.mac_address, d.name);
    }

    // Mutating actions need authentication
    client.login("ABCD1234EFGH").await?;
    client.restart_broadband().await?;
    Ok(())
}
```

## CLI

```
cargo install attrctl               # from crates.io
cargo install --path attrctl        # from a local checkout

attrctl discover
attrctl info
attrctl broadband
attrctl devices --online
attrctl status -o yaml

attrctl --access-code ABCD1234EFGH reboot
attrctl --access-code ABCD1234EFGH restart-broadband
attrctl --access-code ABCD1234EFGH restart-wifi 5
```

Environment variables: `ATTROUTER_IP`, `ATTROUTER_ACCESS_CODE`, `LOG_LEVEL`.

## Testing

Three layers, all wired into the Makefile:

1. **Inline unit tests** (`#[cfg(test)] mod tests` in `attrouter/src/*.rs`)
   cover HTML parsing, MD5 hashing, the vendor/model heuristic, URL
   parsing, and login-page detection. Fast, no network. Uses sanitized
   fixtures from `attrouter/src/testdata/`.

   ```sh
   make test-unit
   ```

2. **Simulator integration tests** (`attrouter/tests/sim_integration.rs`) spin up
   `FakeGateway`, an in-memory axum server that mimics a BGW320 well
   enough to exercise every `Client` method end-to-end — including the
   full login nonce dance, reboot, broadband restart, Wi-Fi restart,
   and clear-and-rescan. Safe to run anywhere.

   ```sh
   make test-sim
   ```

   `make test` runs both layers 1 and 2.

3. **Live integration tests** (`attrouter/tests/live_integration.rs`) drive the
   same `Client` against a real AT&T gateway via `ProtectiveProxy`, an
   axum forwarding proxy. By default the proxy runs in
   `DestructiveMode::Block`: any POST to `/cgi-bin/restart.ha`,
   `/crestart.ha`, `/wrestart.ha`, `/reset.ha`, or `/devices.ha` (clear
   and rescan) is intercepted and answered with a canned success page.
   The real router is never rebooted.

   ```sh
   LIVE_ROUTER=192.168.1.254 \
   LIVE_ROUTER_ACCESS_CODE=ABCD1234EFGH \
     make test-live
   ```

   To actually let destructive operations through (e.g. for acceptance
   testing on a router you are willing to reboot), use
   `test-live-destructive`, which flips the proxy to `Transparent` and
   runs tests serially:

   ```sh
   LIVE_ROUTER=192.168.1.254 \
   LIVE_ROUTER_ACCESS_CODE=ABCD1234EFGH \
     make test-live-destructive
   ```

   > **Warning:** `test-live-destructive` will reboot the gateway. Do not
   > run it against a router you depend on for connectivity — you will
   > lose your network connection mid-test and the run will fail.

   `LIVE_ROUTER` is optional — the test harness auto-discovers the AT&T
   gateway by probing the OS default route, just like `attrctl discover`.
   On a non-AT&T network, live tests skip silently. `make test` (unit +
   sim) never touches the network.

## How it works

The AT&T gateway UI is classic XHTML with a `<tr><th>label</th><td>value</td></tr>`
layout on every status page, form POSTs protected by a per-page nonce, and
MD5-hashed logins that match the router's `login.js`:

    hashpassword = md5(access_code + nonce)

`attrouter` scrapes the tables with `scraper`, manages the `SessionID`
cookie via `reqwest`'s cookie jar, and computes the login hash with
`md-5`. There's no JSON API to call — everything is pulled from the
rendered web pages.

## License

AGPL-3.0-or-later
