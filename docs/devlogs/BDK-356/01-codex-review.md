# Review: `89c6fcb2200ab66394408cac63f6552a860df1fc..c6b5c713020988577694bf207cd0394e1c95f732`

## Findings

1. High: the initializer extracts an unauthenticated tarball as root while TLS verification is disabled.

   `build_http_client()` explicitly turns off certificate validation in the recovery path (`bmc-nix-init/src/init.rs:396-400`). `store::init_store()` then fetches `factory.json`, downloads the selected tarball, and untars it straight into `/` without checking any hash or signature (`bmc-nix/src/store.rs:82-175`). The only trust material in the config, `FactoryServerEntry::known_public_key`, is parsed but never used (`bmc-nix/src/types.rs:170-175`). That means any on-path attacker or compromised mirror can inject arbitrary files and activation scripts into the first-boot recovery flow.

2. High: the generated factory index no longer matches the version string the runtime looks up.

   The runtime now reads the full contents of `/etc/bos_version` (`bmc-nix-init/src/config.rs:57-60`) and `find_tarball_for_version()` requires an exact `bos_version` match (`bmc-nix/src/store.rs:58-62`). However, the build pipeline still hardcodes `bosVersion = "26.02"` and emits that short string into the tarball metadata and generated `factory.json` (`nix/init-artifacts.nix:16`, `nix/init-artifacts.nix:44`, `nix/init-artifacts.nix:57`). The local test helper does the same (`docs/devlogs/BDK-356/test-server/factory.json:5`). As written, a device reporting `2026-03-04-0-8436f26b-26.02` will never match the just-built factory tarball and will fall into the BOS-upgrade/no-upgrade error path instead.

3. High: the captive-portal static file handler allows `..` traversal outside `www_path`.

   `handle_wildcard()` forwards the raw wildcard path to `serve_file()` (`bmc-nix-init/src/server.rs:292-297`), and `serve_file()` blindly opens `www_path.join(file_path)` (`bmc-nix-init/src/server.rs:263-276`). There is no normalization or rejection of `..` segments before opening the file. A client on the recovery AP can therefore request paths such as `/../../etc/passwd` and read files outside the captive-portal asset directory.

4. High: an interrupted initialization can be mistaken for a completed one on the next boot.

   Both the fast path in `bmc-nix-init-openwrt/src/main.rs:65-73` and the main init loop in `bmc-nix-init/src/init.rs:490-493` exit solely on `is_store_ever_initialized()`. On OpenWrt that check only tests whether `/nix/store` and the backing directory exist (`bmc-nix-init-openwrt/src/platform.rs:91-106`). The real completion marker is only written later via `fw_setenv nix_init 1` (`bmc-nix-init-openwrt/src/platform.rs:108-120`), but nothing in this branch reads it, and the activation-sentinel guard is commented out (`bmc-nix-init/src/init.rs:483-488`). If power is lost after extraction creates `/nix/store` but before activation/setup-pending/marker write, the next boot will skip the initializer even though recovery never finished.

5. Medium: a malformed `servers.json` is overwritten without the promised backup.

   `load_servers_config()` documents that an invalid config is backed up before replacement (`bmc-nix-init/src/init.rs:163-169`), but the implementation immediately overwrites the destination via `write_servers_config()` or `write_servers_config_content()` (`bmc-nix-init/src/init.rs:177-193`, `bmc-nix-init/src/init.rs:204-223`). That turns a recoverable typo in `/etc/nix-upgrade/servers.json` into silent data loss and removes the original override that an operator would need for debugging.

## Validation

- `cargo test -p bmc-nix --tests` passed.
- `cargo check -p bmc-nix-init -p bmc-nix-init-openwrt -p bmc-nix-init-mock` did not complete in this environment because `protoc` is not installed, so `bmc-grpc/build.rs` failed first.
