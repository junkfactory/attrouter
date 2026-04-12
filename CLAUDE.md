# Claude Code notes for attrouter

Async Rust client + CLI for AT&T residential gateway routers (BGW210, BGW320,
Humax, Arris). Talks to the gateway's `cgi-bin/*.ha` web UI over plain HTTP
on the LAN. No cloud API.

## Build and test

```sh
make help                  # every target with a one-liner
make test                  # fmt-check + clippy + unit + simulator
make test-live LIVE_ROUTER=192.168.1.254
```

`make test` is the gate — it runs fmt-check, clippy, and all safe tests.
It must stay green. Never use `cargo` commands directly when a Makefile
target exists.

## Layout

Cargo workspace with two member crates.

```
Cargo.toml              workspace manifest (no [package])
attrouter/              library crate
  Cargo.toml
  src/
    lib.rs              module declarations + public re-exports
    client.rs           Client: new(ip), with_base_url(url), get/post_form
    auth.rs             login(access_code) — mirrors login.js hashpwd()
    query.rs            sysinfo, broadband_status, lan_status, devices
    control.rs          restart_device, restart_broadband, restart_wifi, clear_device_list
    discovery.rs        /proc/net/route + vendor/model heuristics
    parse.rs            scraper-based HTML extractors (th_td_map, find_nonce, split_at_hr)
    model.rs            Serialize/Deserialize data types
    error.rs            thiserror Error + Result
    testdata/           sanitized fixtures used by inline parser tests
  tests/
    common/             test-only infrastructure (not public API)
      mod.rs            client_for, env helpers (live_router_upstream, etc.)
      fixtures.rs       include_str! loaders + fresh_nonce()
      simulator.rs      FakeGateway: axum app with session + nonce state
      proxy.rs          ProtectiveProxy: axum forward with Block/Transparent modes
      fixtures/         HTML fixtures (home, login, login_rejected, restart, ...)
    sim_integration.rs  layer 2 (FakeGateway)
    live_integration.rs layer 3 (ProtectiveProxy -> real router, env-gated)
attrctl/                binary crate
  Cargo.toml
  src/
    main.rs             CLI (clap, tokio::main, text/json/yaml output)
```

## Conventions

- Rust 2024 edition. rustc 1.93+.
- Library errors: `thiserror`. CLI errors: `anyhow`.
- Async runtime: `tokio` with `rt-multi-thread` + `macros`.
- HTTP: `reqwest` 0.12 with `cookies` + `rustls-tls`.
- HTML: `scraper` 0.21 with `LazyLock<Selector>` statics.
- Hashing: `md-5` 0.10 + `hex`.
- CLI deps (`clap`, `anyhow`, `serde_json`, `serde_yaml`,
  `tracing-subscriber`) live in the `attrctl` crate. The `attrouter`
  library has no optional features.
- Library dep list is intentionally small. Don't add runtime dependencies
  without discussing first.
- Dev-deps include `axum` 0.8, `tower`, `hyper`, `http`, `http-body-util`,
  `rand`. These exist only to power `attrouter/tests/common/`.

## Gateway protocol

Primary target: **Humax BGW320-500 running firmware 6.34.x** at
`http://192.168.1.254/`. Key properties of the web UI:

- Status pages render `<tr><th>label</th><td>value</td></tr>` tables. The
  parser extracts these into a `HashMap<String, String>` via
  `parse::th_td_map`.
- Every form page embeds a hidden `<input name="nonce" value="..."/>` that
  must be echoed back on POST for CSRF protection. Pick it up with
  `parse::find_nonce`.
- Login (mirrors `login.js`):
  ```
  password     = "*" repeated len(access_code) times
  hashpassword = md5(access_code + nonce) as lowercase hex
  Continue     = "Continue"
  ```
- Session cookie: `SessionID=<hex>; HttpOnly; Path=/`.
- Rejected login re-renders `login.html` with the same `<title>Login</title>`.
  `parse::is_login_page` detects this; `Client::login` returns
  `Error::AuthFailed` on match.
- The devices page groups records with `<tr><td colspan="2"><hr class="reshr"/></td></tr>`
  separator rows inside one big `<table class="table100">`. Use
  `parse::split_at_hr` to split into per-device record groups.
- Destructive endpoints: `/cgi-bin/restart.ha`, `/crestart.ha`,
  `/wrestart.ha?[12]`, `/reset.ha`. All require a prior `login()` and all
  use the nonce dance.

## Test infrastructure

Three layers — see README.md "Testing" for the user-facing write-up.

**Auto-discovery**: `live_router_upstream()` in `attrouter/tests/common/mod.rs` calls
`attrouter::discover()` when `LIVE_ROUTER` is unset. Result is cached for
the test binary's lifetime via `tokio::sync::OnceCell`. On an AT&T network,
`make test-live` (and bare `cargo test --test live_integration`) auto-detects
the gateway and runs. On non-AT&T networks, live tests skip silently.
Destructive tests `.expect()` the router to be reachable — they hard-fail
instead of skip.

**FakeGateway** (`attrouter/tests/common/simulator.rs`) is an axum app that:
- Binds `127.0.0.1:0` and tracks per-session cookies, authentication, and
  the last-issued nonce.
- Renders sanitized fixtures with fresh `{{NONCE}}` substitution via
  `fixtures::render`.
- Validates POST nonces against stored values and invalidates on use (single
  use), which approximates real CSRF behaviour.
- Counts reboots / broadband restarts / wifi restarts for test assertions.
- Access code defaults to `"letmein"`; change via `sim.set_access_code()`.

**ProtectiveProxy** (`attrouter/tests/common/proxy.rs`) sits in front of a real router:
- Default mode `DestructiveMode::Block` intercepts POSTs whose path starts
  with any entry in `DESTRUCTIVE_PATHS` (`restart`, `crestart`, `wrestart`,
  `reset`, `devices`) and returns the canned `restart_accepted.html`.
- `DestructiveMode::Transparent` forwards everything as-is. Only reachable
  via `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1`.
- Tracks `intercepted_count` and `forwarded_count` for test assertions.

## Fixture sanitization (hard rule)

Fixtures under `attrouter/src/testdata/` and `attrouter/tests/common/fixtures/` must NEVER
contain real data. When recapturing from a live router, scrub:

| Field                | Replace with                             |
|----------------------|------------------------------------------|
| MAC addresses        | `00:11:22:33:44:XX`                      |
| Serial numbers       | `TEST1234567890`                         |
| Hardware version     | `02000000000000`                         |
| WAN IPv4             | RFC 5737 (`192.0.2.*`)                   |
| WAN IPv6             | RFC 3849 (`2001:db8::*`)                 |
| Link-local           | `fe80::1`, `fe80::2`, `fe80::254`        |
| Nonces               | literal `{{NONCE}}` placeholder          |
| Session cookies      | discard                                  |
| Host names           | `host-alpha`, `host-beta`, `host-gamma`  |

Inline-test assertions use these same replacement values, so changing the
sanitization table is an API-breaking change for the tests.

## Environment variables

Runtime (for `attrctl`):
- `ATTROUTER_IP` — gateway IP, otherwise auto-discover
- `ATTROUTER_ACCESS_CODE` — device access code
- `LOG_LEVEL` — `debug`, `trace`, etc.

Test (for `attrouter/tests/live_integration.rs`):
- `LIVE_ROUTER=<ip>` — override auto-discovery (optional; the test harness
  probes the default gateway via `attrouter::discover()` automatically)
- `LIVE_ROUTER_ACCESS_CODE=<code>` — for tests that authenticate
- `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1` — flip the proxy to Transparent mode.
  Router will actually reboot. Missing-router is a hard test failure in
  this mode, not a skip. **Do not use on a router you depend on for
  connectivity — the reboot will kill your network connection mid-test.**

## Safety posture

- Never commit a real device access code, serial number, or MAC.
- Never bypass the `ProtectiveProxy` intercept without an explicit
  `LIVE_ROUTER_ALLOW_DESTRUCTIVE=1` opt-in.
- `make test` (with no env vars) must never reach the live router.
- When fixing a parse bug, add an inline test with a sanitized fixture
  snippet that reproduces it. Don't assume the live router still serves
  the same HTML — fixtures are the source of truth.

## Release process

Both crates publish to crates.io. `attrouter` must be published first
because `attrctl` depends on it.

1. Bump `version` in `attrouter/Cargo.toml` and `attrctl/Cargo.toml`.
2. Update the `attrouter` version in `attrctl/Cargo.toml` `[dependencies]`
   to match.
3. `make test` — must pass.
4. Commit the version bump.
5. `git tag v<VERSION>` (e.g. `v0.2.0`).
6. `git push && git push --tags`
7. `cargo publish -p attrouter`
8. `cargo publish -p attrctl`
