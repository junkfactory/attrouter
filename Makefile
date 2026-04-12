.DEFAULT_GOAL := help

CARGO ?= cargo

.PHONY: help
help:
	@echo "attrouter — async Rust client for AT&T residential gateways"
	@echo ""
	@echo "Build:"
	@echo "  make build                 cargo build --workspace"
	@echo "  make release               cargo build --workspace --release"
	@echo "  make install               cargo install --path attrctl"
	@echo "  make clean                 cargo clean"
	@echo "  make doc                   cargo doc -p attrouter --no-deps --open"
	@echo ""
	@echo "Test (three layers):"
	@echo "  make test                  fmt-check + clippy + unit + simulator"
	@echo "  make test-unit             inline unit tests only (layer 1)"
	@echo "  make test-sim              simulator integration only (layer 2)"
	@echo "  make test-live             live-router tests via ProtectiveProxy"
	@echo "                             (requires LIVE_ROUTER=<ip>; NEVER reboots)"
	@echo "  make test-live-destructive actually reboots the router, run serially"
	@echo "                             (requires LIVE_ROUTER + LIVE_ROUTER_ACCESS_CODE)"
	@echo ""
	@echo "  *** WARNING: test-live-destructive will reboot the gateway. ***"
	@echo "  *** Do not run against a router you depend on for connectivity. ***"
	@echo ""
	@echo "Quality:"
	@echo "  make clippy                cargo clippy --workspace --all-targets -D warnings"
	@echo "  make fmt                   cargo fmt --all"
	@echo "  make fmt-check             cargo fmt --all --check"
	@echo ""
	@echo "Environment variables:"
	@echo "  LIVE_ROUTER                gateway IP for live tests (e.g. 192.168.1.254)"
	@echo "  LIVE_ROUTER_ACCESS_CODE    12-char device access code on the router label"
	@echo "  LIVE_ROUTER_ALLOW_DESTRUCTIVE=1"
	@echo "                             switch the proxy to Transparent mode"
	@echo "  ATTROUTER_IP               runtime gateway IP for attrctl"
	@echo "  ATTROUTER_ACCESS_CODE      runtime access code for attrctl"

.PHONY: build
build:
	$(CARGO) build --workspace

.PHONY: release
release:
	$(CARGO) build --workspace --release

.PHONY: install
install:
	$(CARGO) install --path attrctl

.PHONY: doc
doc:
	$(CARGO) doc -p attrouter --no-deps --open

.PHONY: clean
clean:
	$(CARGO) clean

.PHONY: test
test: fmt-check clippy test-unit test-sim

.PHONY: test-unit
test-unit:
	$(CARGO) test -p attrouter --lib
	$(CARGO) test -p attrouter --doc

.PHONY: test-sim
test-sim:
	$(CARGO) test -p attrouter --test sim_integration

.PHONY: test-live
test-live: build
	@if [ -n "$$LIVE_ROUTER" ]; then \
	  echo "[test-live] using LIVE_ROUTER=$$LIVE_ROUTER"; \
	elif ./target/debug/attrctl discover >/dev/null 2>&1; then \
	  echo "[test-live] auto-discovered: $$(./target/debug/attrctl discover 2>/dev/null | head -1)"; \
	else \
	  echo "[test-live] no AT&T gateway reachable — live tests will skip"; \
	fi
	$(CARGO) test -p attrouter --test live_integration

.PHONY: test-live-destructive
test-live-destructive: build
	@if [ -z "$$LIVE_ROUTER_ACCESS_CODE" ]; then \
	  echo "error: LIVE_ROUTER_ACCESS_CODE is required for destructive tests"; \
	  exit 1; \
	fi
	@if [ -n "$$LIVE_ROUTER" ]; then \
	  echo "[test-live-destructive] using explicit LIVE_ROUTER=$$LIVE_ROUTER"; \
	elif ./target/debug/attrctl discover >/dev/null 2>&1; then \
	  echo "[test-live-destructive] auto-discovered: $$(./target/debug/attrctl discover 2>/dev/null | head -1)"; \
	else \
	  echo "error: no AT&T gateway reachable."; \
	  echo "       connect to an AT&T-served network, or set LIVE_ROUTER=<ip>."; \
	  exit 1; \
	fi
	@echo ""
	@echo "!! WARNING !!"
	@echo "!! This will ACTUALLY reboot the gateway. !!"
	@echo "!! Ctrl-C within 5s to abort. !!"
	@echo ""
	@sh -c 'for i in 5 4 3 2 1; do printf "%s... " $$i; sleep 1; done; echo'
	LIVE_ROUTER_ALLOW_DESTRUCTIVE=1 $(CARGO) test -p attrouter --test live_integration -- \
		--test-threads=1 --nocapture

.PHONY: clippy
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: fmt
fmt:
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check:
	$(CARGO) fmt --all --check
