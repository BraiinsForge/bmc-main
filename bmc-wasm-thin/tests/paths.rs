// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::path::Path;
use std::time::Duration;

use bmc_wasm_thin::args::{Config, RawArgs};
use bmc_wasm_thin::paths::{
    derive_lockfile_path, derive_owner_record_path, resolve_wayland_display_path,
};
use bmc_wasm_thin_protocol::{
    default_lockfile_path, default_owner_record_path, default_socket_path,
    socket_path_for_sdk_major,
};

#[test]
fn socket_defaults_follow_runtime_sdk_major() {
    // Derives the major rather than pinning the version: this exercises the
    // socket-path scheme, not the SDK version, so minor/patch bumps shouldn't touch it.
    let major = bmc_wasm_protocol::SDK_VERSION.0;
    let socket = format!("/run/bmc/wasm-host-sdk-v{major}.sock");
    let lock = format!("/run/bmc/wasm-host-sdk-v{major}.lock");
    let owner = format!("/run/bmc/wasm-host-sdk-v{major}.owner");
    assert_eq!(socket_path_for_sdk_major(major), default_socket_path());
    assert_eq!(default_socket_path(), Path::new(&socket));
    assert_eq!(default_lockfile_path(), Path::new(&lock));
    assert_eq!(default_owner_record_path(), Path::new(&owner));
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
fn owner_record_path_follows_socket_override() {
    assert_eq!(
        derive_owner_record_path(Path::new("/tmp/test/host.sock")),
        Path::new("/tmp/test/host.owner"),
    );
    assert_eq!(
        derive_owner_record_path(Path::new("/tmp/test/host")),
        Path::new("/tmp/test/host.owner"),
    );
    assert_eq!(
        derive_owner_record_path(Path::new("/tmp/test/host.socket")),
        Path::new("/tmp/test/host.socket.owner"),
    );
}

#[test]
fn sdk_major_owner_records_are_independent() {
    let first = derive_owner_record_path(&socket_path_for_sdk_major(0));
    let second = derive_owner_record_path(&socket_path_for_sdk_major(1));
    assert_ne!(first, second);
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
        asset_root: None,
        host_socket: None,
        host_bin: None,
        host_wait_ms: None,
        ack_wait_ms: None,
    };
    let config = Config::from_raw_with_env(raw, &[]).expect("BUG: valid raw args should normalize");
    assert_eq!(config.host_socket, default_socket_path());
    assert_eq!(config.lockfile, default_lockfile_path());
    assert_eq!(config.owner_record, default_owner_record_path());
    assert_eq!(config.host_wait, Duration::from_secs(10));
    assert_eq!(config.ack_wait, Duration::from_secs(10));
}

#[test]
fn production_config_entrypoint_reads_explicit_env_overrides() {
    let raw = RawArgs {
        wasm: Path::new("/tmp/widget.wasm").to_path_buf(),
        asset_root: None,
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
