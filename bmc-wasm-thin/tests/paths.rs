// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;
use std::time::Duration;

use bmc_wasm_thin::args::{Config, RawArgs};
use bmc_wasm_thin::paths::{derive_lockfile_path, resolve_wayland_display_path};
use bmc_wasm_thin_protocol::{
    default_lockfile_path, default_socket_path, socket_path_for_sdk_major,
};

#[test]
fn socket_defaults_follow_runtime_sdk_major() {
    // Derives the major rather than pinning the version: this exercises the
    // socket-path scheme, not the SDK version, so minor/patch bumps shouldn't touch it.
    let major = bmc_wasm_protocol::SDK_VERSION.0;
    let socket = format!("/run/bmc/wasm-host-sdk-v{major}.sock");
    let lock = format!("/run/bmc/wasm-host-sdk-v{major}.lock");
    assert_eq!(socket_path_for_sdk_major(major), default_socket_path());
    assert_eq!(default_socket_path(), Path::new(&socket));
    assert_eq!(default_lockfile_path(), Path::new(&lock));
}

#[test]
fn lockfile_path_follows_socket_override() {
    assert_eq!(
        derive_lockfile_path(Path::new("/tmp/test/host.sock")),
        Path::new("/tmp/test/host.lock"),
    );
    assert_eq!(
        derive_lockfile_path(Path::new("/tmp/test/host")),
        Path::new("/tmp/test/host.lock"),
    );
    assert_eq!(
        derive_lockfile_path(Path::new("/tmp/test/host.socket")),
        Path::new("/tmp/test/host.socket.lock"),
    );
}

#[test]
fn wayland_display_path_resolution_matches_widget_manager_defaults() {
    assert_eq!(
        resolve_wayland_display_path(Some("/abs/socket"), Some("/run/user/1000")),
        Path::new("/abs/socket"),
    );
    assert_eq!(
        resolve_wayland_display_path(Some("wayland-1"), Some("/run/user/1000")),
        Path::new("/run/user/1000/wayland-1"),
    );
    assert_eq!(
        resolve_wayland_display_path(None, Some("/tmp/run-test")),
        Path::new("/tmp/run-test/wayland-0"),
    );
    assert_eq!(
        resolve_wayland_display_path(None, None),
        Path::new("/tmp/run/wayland-0"),
    );
}

#[test]
fn config_defaults_use_protocol_paths() {
    let raw = RawArgs {
        wasm: Path::new("/tmp/widget.wasm").to_path_buf(),
        host_socket: None,
        host_bin: None,
        host_wait_ms: None,
        ack_wait_ms: None,
    };
    let config = Config::from_raw_with_env(raw, &[]).expect("BUG: valid raw args should normalize");
    assert_eq!(config.host_socket, default_socket_path());
    assert_eq!(config.lockfile, default_lockfile_path());
    assert_eq!(config.host_wait, Duration::from_secs(10));
    assert_eq!(config.ack_wait, Duration::from_secs(10));
}

#[test]
fn production_config_entrypoint_reads_explicit_env_overrides() {
    let raw = RawArgs {
        wasm: Path::new("/tmp/widget.wasm").to_path_buf(),
        host_socket: None,
        host_bin: None,
        host_wait_ms: None,
        ack_wait_ms: None,
    };
    let config = Config::from_raw_with_env(
        raw,
        &[
            ("BMC_WASM_HOST_WAIT_MS", "123".to_owned()),
            ("BMC_WASM_HOST_ACK_WAIT_MS", "456".to_owned()),
        ],
    )
    .expect("BUG: valid env overrides should normalize");
    assert_eq!(config.host_wait, Duration::from_millis(123));
    assert_eq!(config.ack_wait, Duration::from_millis(456));
}
