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

//! `WidgetSlot`-level tests over a stub surface: the update-delivery and
//! dirty-render-gating paths as the main loop drives them, not their
//! extracted helpers.

use std::cell::Cell;
use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use bmc_system_overlay::{Snapshot, SnapshotVersion, VersionedSnapshot};
use bmc_wasm_host::lifecycle::{LifecycleEgl, LifecycleSurface};
use bmc_wasm_host::render_target::{
    RenderTarget, RenderTargetCleanup, RenderTargetError, RenderTargetFactory,
};
use bmc_wasm_host::slot::{SlotSurface, WidgetSlot};
use bmc_wasm_runtime::{RuntimeConfig, RuntimeDisplayInfo, WasmWidgetRuntime};
use bmc_widget::surface::{PollOutcome, ReleasedBuffer, WidgetEvent, WidgetSurface};
use bmc_widget_protocol::{ActionPayload, CredentialSecrets, SettingUpdate};

#[derive(Default)]
struct StubSurface {
    needs_render: bool,
    mark_needs_render_calls: usize,
    queued_events: Vec<WidgetEvent>,
}

impl WidgetSurface for StubSurface {
    fn running(&self) -> bool {
        true
    }

    fn request_shutdown(&mut self) {
        unimplemented!("stub: request_shutdown unused in these tests")
    }

    fn width(&self) -> u32 {
        320
    }

    fn height(&self) -> u32 {
        240
    }

    fn take_size_changed(&mut self) -> bool {
        false
    }

    fn needs_render(&self) -> bool {
        self.needs_render
    }

    fn take_render_requested(&mut self) -> bool {
        std::mem::take(&mut self.needs_render)
    }

    fn mark_needs_render(&mut self) {
        self.needs_render = true;
        self.mark_needs_render_calls += 1;
    }

    fn frame_count(&self) -> u32 {
        0
    }

    fn blocking_dispatch(&mut self) -> anyhow::Result<()> {
        unimplemented!("stub: blocking_dispatch unused in these tests")
    }

    fn poll_dispatch(&mut self, _timeout_ms: i32) -> anyhow::Result<PollOutcome> {
        Ok(PollOutcome::Events)
    }

    fn request_frame(&self) {
        unimplemented!("stub: request_frame unused in these tests")
    }

    fn submit_buffer(
        &mut self,
        _info: &bmc_widget::egl::DmaBufInfo,
        _slot: usize,
        _request_frame: bool,
    ) -> anyhow::Result<()> {
        unimplemented!("stub: submit_buffer unused in these tests")
    }

    fn invalidate_cached_buffers(&mut self) {
        unimplemented!("stub: invalidate_cached_buffers unused in these tests")
    }

    fn invalidate_cached_buffer_slots(&mut self, _slots: &[usize]) {
        unimplemented!("stub: invalidate_cached_buffer_slots unused in these tests")
    }

    fn drain_released_slots(&mut self) -> Vec<usize> {
        Vec::new()
    }

    fn drain_events(&mut self) -> Vec<WidgetEvent> {
        std::mem::take(&mut self.queued_events)
    }
}

impl LifecycleSurface for StubSurface {
    fn as_deck_widget_surface(&self) -> &bmc_widget::surface::DeckWidgetSurfaceClient {
        unimplemented!("stub: as_deck_widget_surface unused in these tests")
    }

    fn mint_wl_buffer(
        &mut self,
        _dmabuf: &bmc_widget::egl::DmaBufInfo,
        _slot: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        unimplemented!("stub: mint_wl_buffer unused in these tests")
    }

    fn destroy_minted_wl_buffer(&mut self, _buffer: wayland_client::protocol::wl_buffer::WlBuffer) {
        unimplemented!("stub: destroy_minted_wl_buffer unused in these tests")
    }
}

impl SlotSurface for StubSurface {
    fn request_action(&self, _action: &ActionPayload) -> anyhow::Result<()> {
        Ok(())
    }

    fn submit_buffer_with_wl_buffer(
        &self,
        _info: &bmc_widget::egl::DmaBufInfo,
        _buffer: &wayland_client::protocol::wl_buffer::WlBuffer,
        _request_frame: bool,
    ) -> anyhow::Result<()> {
        unimplemented!("stub: submit_buffer_with_wl_buffer unused in these tests")
    }

    fn flush(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        Vec::new()
    }

    fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        unimplemented!("stub: fd unused in these tests")
    }
}

struct StubEgl;
impl LifecycleEgl for StubEgl {
    fn as_egl_context(&self) -> &bmc_widget::egl::EglContext {
        unimplemented!("stub: StubFactory never touches EGL")
    }
}

#[derive(Default)]
struct StubFactory {
    allocations: Cell<usize>,
    releases: Cell<usize>,
}

impl RenderTargetFactory for StubFactory {
    fn allocate(
        &self,
        _: &dyn LifecycleEgl,
        _: &mut dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError> {
        self.allocations.set(self.allocations.get() + 1);
        Ok(RenderTarget::new_stub(width, height))
    }

    fn destroy(&self, _: RenderTarget, _: &dyn LifecycleEgl, _: &mut dyn LifecycleSurface) {}

    fn destroy_released_slots(
        &self,
        _: &mut RenderTarget,
        _: &dyn LifecycleEgl,
        _: &mut dyn LifecycleSurface,
    ) -> RenderTargetCleanup {
        self.releases.set(self.releases.get() + 1);
        RenderTargetCleanup::Complete
    }
}

fn network_probe_widget_wat() -> String {
    include_str!("../../bmc-wasm-runtime/tests/fixtures/network_probe.wat").replace(
        "__SDK_VERSION__",
        &bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION).to_string(),
    )
}

fn update_probe_widget_wat(hook: &str, request_frame: bool) -> String {
    include_str!("../../bmc-wasm-runtime/tests/fixtures/update_probe.wat")
        .replace("__UPDATE_HOOK__", hook)
        .replace(
            "__SDK_VERSION__",
            &bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION).to_string(),
        )
        .replace(
            "__REQUEST_FRAME__",
            if request_frame {
                "call $host_request_frame"
            } else {
                ""
            },
        )
}

fn credential_snapshot_probe_widget_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_request_frame" (func $host_request_frame))
      (import "env" "host_credentials_snapshot"
        (func $host_credentials_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (global $update_count (mut i32) (i32.const 0))
      (global $type_first_byte (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64) i64.const {})
      (func (export "render") (param i32))

      (func (export "on_credentials_update")
        i32.const 0
        i32.const 64
        call $host_credentials_snapshot
        drop
        ;; Packed view: count (4) + "pool" length (2) + bytes (4) + type length (2).
        i32.const 12
        i32.load8_u
        global.set $type_first_byte
        global.get $update_count
        i32.const 1
        i32.add
        global.set $update_count
        call $host_request_frame)

      (func (export "update_count") (result i32) global.get $update_count)
      (func (export "type_first_byte") (result i32) global.get $type_first_byte))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

fn hookless_widget_wat() -> String {
    format!(
        r#"
    (module
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64) i64.const {})
      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

fn test_slot(wat: &str) -> WidgetSlot<StubSurface> {
    test_slot_with_factory(wat, Rc::new(StubFactory::default()))
}

fn test_slot_with_factory(
    wat: &str,
    factory: Rc<dyn RenderTargetFactory>,
) -> WidgetSlot<StubSurface> {
    let wasm = wat::parse_str(wat).expect("BUG: test WAT must parse");
    let runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 320,
            height: 240,
            shape: bmc_wasm_protocol::DisplayShape::Rectangular,
            dpi: 1,
        },
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: test runtime must construct");
    let (control_socket, _peer) = UnixStream::pair().expect("BUG: socketpair must construct");
    let (_led_tx, led_rx) = mpsc::channel();
    WidgetSlot::from_parts(
        StubSurface::default(),
        runtime,
        factory,
        control_socket,
        led_rx,
    )
}

fn online_snapshot(ssid: &str, last_octet: u8, signal_dbm: i32) -> Snapshot {
    Snapshot {
        ipv4: Some(Ipv4Addr::new(10, 0, 0, last_octet)),
        station_ipv4: Some(Ipv4Addr::new(10, 0, 0, last_octet)),
        station_ssid: Some(ssid.to_owned()),
        wifi_signal_dbm: Some(signal_dbm),
    }
}

fn credential_view(
    slot: &str,
    type_id: &str,
    account_name: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut credentials = serde_json::Map::new();
    credentials.insert(
        slot.to_owned(),
        serde_json::json!({ "type": type_id, "account": account_name }),
    );
    credentials
}

fn credential_secrets(slot: &str, field: &str, value: &str) -> CredentialSecrets {
    let mut fields = serde_json::Map::new();
    fields.insert(
        field.to_owned(),
        serde_json::Value::String(value.to_owned()),
    );
    let mut secrets = serde_json::Map::new();
    secrets.insert(slot.to_owned(), serde_json::json!({ "fields": fields }));
    CredentialSecrets::new(secrets)
}

fn dispatch_events(
    slot: &mut WidgetSlot<StubSurface>,
    events: impl IntoIterator<Item = WidgetEvent>,
) {
    slot.surface.queued_events.extend(events);
    slot.dispatch_wayland_events()
        .expect("BUG: stub dispatch cannot fail");
}

fn dispatch_event(slot: &mut WidgetSlot<StubSurface>, event: WidgetEvent) {
    dispatch_events(slot, [event]);
}

fn enter_state(slot: &mut WidgetSlot<StubSurface>, state: bmc_widget_protocol::LifecycleState) {
    dispatch_event(slot, WidgetEvent::Lifecycle(state));
    slot.apply_lifecycle(Instant::now(), &StubEgl);
}

// ── parameter updates ──────────────────────────────────────────────

#[test]
fn params_update_hook_requests_frame_without_marking_the_surface_dirty() {
    let wat = update_probe_widget_wat("on_params_update", true);
    let mut slot = test_slot(&wat);

    dispatch_event(&mut slot, WidgetEvent::ParamUpdate(serde_json::Map::new()));

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "parameter delivery must invoke on_params_update"
    );
    assert!(
        slot.runtime.wants_next_frame(),
        "request_frame from on_params_update must reach the runtime scheduler"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "parameter delivery must leave surface rendering under widget ownership"
    );
}

#[test]
fn params_update_hook_can_decline_render_without_dirtying_the_surface() {
    let wat = update_probe_widget_wat("on_params_update", false);
    let mut slot = test_slot(&wat);

    dispatch_event(&mut slot, WidgetEvent::ParamUpdate(serde_json::Map::new()));

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "parameter delivery must invoke on_params_update"
    );
    assert!(
        !slot.runtime.wants_next_frame(),
        "a hook that declines a frame must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "a hook that declines a frame must not be overridden by surface dirtiness"
    );
}

#[test]
fn params_update_without_hook_does_not_schedule_or_dirty() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_event(&mut slot, WidgetEvent::ParamUpdate(serde_json::Map::new()));

    assert!(
        !slot.runtime.wants_next_frame(),
        "parameter delivery without a hook must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "parameter delivery without a hook must not dirty the surface"
    );
}

// ── system-setting updates ────────────────────────────────────────────

#[test]
fn system_update_hook_requests_frame_without_marking_the_surface_dirty() {
    let wat = update_probe_widget_wat("on_system_update", true);
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::Setting(SettingUpdate::NightMode(true)),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "system delivery must invoke on_system_update"
    );
    assert!(
        slot.runtime.wants_next_frame(),
        "request_frame from on_system_update must reach the runtime scheduler"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "system delivery must leave surface rendering under widget ownership"
    );
}

#[test]
fn system_update_hook_can_decline_render_without_dirtying_the_surface() {
    let wat = update_probe_widget_wat("on_system_update", false);
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::Setting(SettingUpdate::NightMode(true)),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "system delivery must invoke on_system_update"
    );
    assert!(
        !slot.runtime.wants_next_frame(),
        "a hook that declines a frame must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "a hook that declines a frame must not be overridden by surface dirtiness"
    );
}

#[test]
fn system_update_without_hook_does_not_schedule_or_dirty() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::Setting(SettingUpdate::NightMode(true)),
    );

    assert!(
        !slot.runtime.wants_next_frame(),
        "system delivery without a hook must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "system delivery without a hook must not dirty the surface"
    );
}

// ── credential updates ─────────────────────────────────────────────

#[test]
fn public_credential_update_hook_requests_frame_without_dirtying_the_surface() {
    let wat = update_probe_widget_wat("on_credentials_update", true);
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::CredentialsUpdate(credential_view("pool", "braiins-pool", "Primary")),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "public credential delivery must invoke on_credentials_update"
    );
    assert_eq!(slot.credentials.type_of("pool"), Some("braiins-pool"));
    assert!(
        slot.runtime.wants_next_frame(),
        "request_frame from on_credentials_update must reach the runtime scheduler"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "public credential delivery must leave rendering under widget ownership"
    );
}

#[test]
fn secret_credential_update_hook_can_decline_render_without_dirtying_the_surface() {
    let wat = update_probe_widget_wat("on_credentials_update", false);
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::SecretsUpdate(credential_secrets("pool", "token", "first-token")),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "secret credential delivery must invoke on_credentials_update"
    );
    assert_eq!(
        slot.credential_secrets.field("pool", "token"),
        Some("first-token")
    );
    assert!(
        !slot.runtime.wants_next_frame(),
        "a hook that declines a frame must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "a hook that declines a frame must not be overridden by surface dirtiness"
    );
}

#[test]
fn credential_update_without_hook_does_not_schedule_or_dirty() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::CredentialsUpdate(credential_view("pool", "braiins-pool", "Primary")),
    );

    assert!(
        !slot.runtime.wants_next_frame(),
        "credential delivery without a hook must leave the runtime scheduler idle"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "credential delivery without a hook must not dirty the surface"
    );
}

#[test]
fn credential_events_in_one_drain_deliver_one_latest_combined_snapshot() {
    let wat = credential_snapshot_probe_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_events(
        &mut slot,
        [
            WidgetEvent::CredentialsUpdate(credential_view("pool", "generic-token", "Old account")),
            WidgetEvent::SecretsUpdate(credential_secrets("pool", "token", "old-token")),
            WidgetEvent::CredentialsUpdate(credential_view(
                "pool",
                "braiins-pool",
                "Latest account",
            )),
            WidgetEvent::SecretsUpdate(credential_secrets("pool", "token", "latest-token")),
        ],
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(1),
        "one event drain must invoke on_credentials_update once"
    );
    assert_eq!(
        slot.runtime.call_export_i32("type_first_byte"),
        Some(i32::from(b'b')),
        "the hook must observe the latest public snapshot in the runtime"
    );
    assert_eq!(slot.credentials.type_of("pool"), Some("braiins-pool"));
    assert_eq!(
        slot.credential_secrets.field("pool", "token"),
        Some("latest-token")
    );
    assert!(
        slot.runtime.wants_next_frame(),
        "the coalesced hook's request_frame must reach the runtime scheduler"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "coalesced credential delivery must not dirty the surface"
    );
}

#[test]
fn secret_update_preserves_public_credentials_from_previous_drain() {
    let wat = credential_snapshot_probe_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::CredentialsUpdate(credential_view("pool", "braiins-pool", "Primary")),
    );
    dispatch_event(
        &mut slot,
        WidgetEvent::SecretsUpdate(credential_secrets("pool", "token", "first-token")),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(2),
        "separate event drains must deliver each credential update"
    );
    assert_eq!(
        slot.runtime.call_export_i32("type_first_byte"),
        Some(i32::from(b'b')),
        "a secret-only drain must preserve the public snapshot delivered earlier"
    );
    assert_eq!(slot.credentials.type_of("pool"), Some("braiins-pool"));
    assert_eq!(
        slot.credential_secrets.field("pool", "token"),
        Some("first-token")
    );
    assert_eq!(slot.surface.mark_needs_render_calls, 0);
}

#[test]
fn public_update_preserves_secrets_from_previous_drain() {
    let wat = credential_snapshot_probe_widget_wat();
    let mut slot = test_slot(&wat);

    dispatch_event(
        &mut slot,
        WidgetEvent::SecretsUpdate(credential_secrets("pool", "token", "first-token")),
    );
    dispatch_event(
        &mut slot,
        WidgetEvent::CredentialsUpdate(credential_view("pool", "braiins-pool", "Primary")),
    );

    assert_eq!(
        slot.runtime.call_export_i32("update_count"),
        Some(2),
        "separate event drains must deliver each credential update"
    );
    assert_eq!(slot.credentials.type_of("pool"), Some("braiins-pool"));
    assert_eq!(
        slot.credential_secrets.field("pool", "token"),
        Some("first-token"),
        "a public-only drain must preserve the secret snapshot delivered earlier"
    );
    assert_eq!(slot.surface.mark_needs_render_calls, 0);
}

// ── refresh_network ────────────────────────────────────────────────

#[test]
fn network_change_is_delivered_without_marking_the_surface_dirty() {
    let wat = network_probe_widget_wat();
    let mut slot = test_slot(&wat);

    slot.refresh_network_from(|seen| {
        assert!(seen.is_none(), "first poll hands the prober no version");
        Some(VersionedSnapshot {
            version: SnapshotVersion::FIRST,
            snapshot: online_snapshot("deck-net", 7, -50),
        })
    });

    assert_eq!(
        slot.runtime.call_export_i32("network_count"),
        Some(1),
        "an SSID/IP change must reach the widget's on_network_update hook"
    );
    assert!(
        slot.runtime.wants_next_frame(),
        "the hook's request_frame is how the change reaches the screen"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "refresh_network must never mark the surface dirty — the host-forced \
         render on network changes is the BDK-658 bug; the widget decides \
         via request_frame, like on_touch"
    );
}

#[test]
fn signal_only_bump_consumes_the_version_but_never_wakes_the_widget() {
    let wat = network_probe_widget_wat();
    let mut slot = test_slot(&wat);

    slot.refresh_network_from(|_| {
        Some(VersionedSnapshot {
            version: SnapshotVersion::FIRST,
            snapshot: online_snapshot("deck-net", 7, -50),
        })
    });
    assert_eq!(slot.runtime.call_export_i32("network_count"), Some(1));

    let bumped = SnapshotVersion::FIRST.next();
    slot.refresh_network_from(|seen| {
        assert_eq!(
            seen,
            Some(SnapshotVersion::FIRST),
            "the delivered version must be handed back so an unchanged \
             network re-polls free"
        );
        Some(VersionedSnapshot {
            version: bumped,
            snapshot: online_snapshot("deck-net", 7, -62),
        })
    });

    assert_eq!(
        slot.runtime.call_export_i32("network_count"),
        Some(1),
        "a dBm-only snapshot bump is invisible to widgets and must not fire \
         the hook — per-second RSSI jitter waking every widget is the \
         BDK-658 bug"
    );
    assert_eq!(
        slot.surface.mark_needs_render_calls, 0,
        "no dirty flag on any network path"
    );

    slot.refresh_network_from(|seen| {
        assert_eq!(
            seen,
            Some(bumped),
            "a signal-only bump must still be consumed, or every main-loop \
             iteration re-clones the same snapshot"
        );
        None
    });
}

#[test]
fn unchanged_network_delivers_nothing() {
    let wat = network_probe_widget_wat();
    let mut slot = test_slot(&wat);

    slot.refresh_network_from(|_| None);

    assert_eq!(
        slot.runtime.call_export_i32("network_count"),
        Some(0),
        "no snapshot bump — nothing may reach the widget"
    );
    assert!(!slot.runtime.wants_next_frame());
    assert_eq!(slot.surface.mark_needs_render_calls, 0);
}

// ── lifecycle resource ownership ────────────────────────────────────

#[test]
fn render_target_is_retained_until_the_slot_reaches_dormant() {
    let factory = Rc::new(StubFactory::default());
    let mut slot = test_slot_with_factory(&hookless_widget_wat(), factory.clone());

    assert!(
        slot.runtime.renderer_assets_are_dormant_for_test(),
        "a newly constructed slot must start with dormant renderer assets"
    );

    for state in [
        bmc_widget_protocol::LifecycleState::Prepared,
        bmc_widget_protocol::LifecycleState::Entering,
        bmc_widget_protocol::LifecycleState::Visible,
        bmc_widget_protocol::LifecycleState::Leaving,
    ] {
        enter_state(&mut slot, state);
        assert!(
            slot.render_target.is_some(),
            "{state:?} must retain the render target"
        );
        assert_eq!(
            factory.allocations.get(),
            1,
            "moving between renderable states must reuse the original target"
        );
        assert_eq!(
            factory.releases.get(),
            0,
            "{state:?} must not release the render target"
        );
        assert!(
            !slot.runtime.renderer_assets_are_dormant_for_test(),
            "{state:?} must keep the runtime active"
        );
    }

    dispatch_event(
        &mut slot,
        WidgetEvent::Lifecycle(bmc_widget_protocol::LifecycleState::Dormant),
    );

    assert!(
        slot.render_target.is_some(),
        "receiving Dormant must not release the target before lifecycle application"
    );
    assert_eq!(
        factory.releases.get(),
        0,
        "receiving Dormant must not release the render target"
    );
    assert!(
        !slot.runtime.renderer_assets_are_dormant_for_test(),
        "receiving Dormant must not mark the runtime before lifecycle application"
    );

    slot.apply_lifecycle(Instant::now(), &StubEgl);

    assert!(
        slot.render_target.is_none(),
        "Dormant must release the render target"
    );
    assert_eq!(
        factory.allocations.get(),
        1,
        "entering Dormant must not allocate a replacement target"
    );
    assert_eq!(
        factory.releases.get(),
        1,
        "reaching Dormant must release the original target exactly once"
    );
    assert!(
        slot.runtime.renderer_assets_are_dormant_for_test(),
        "the runtime must become dormant only after the slot applies Dormant"
    );
}

#[test]
fn waking_into_prepared_acquires_a_target_and_requests_a_guest_frame() {
    let factory = Rc::new(StubFactory::default());
    let mut slot = test_slot_with_factory(&hookless_widget_wat(), factory.clone());

    assert!(
        slot.render_target.is_none(),
        "a Dormant slot must start without a render target"
    );
    assert!(
        !slot.runtime.wants_next_frame(),
        "no guest frame may be pending before the first wake edge"
    );

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Prepared);

    assert!(
        slot.render_target.is_some(),
        "Prepared must acquire a render target before waking the runtime"
    );
    assert_eq!(
        factory.allocations.get(),
        1,
        "the Dormant-to-Prepared edge must allocate exactly one target"
    );
    assert!(
        slot.runtime.wants_next_frame(),
        "the wake edge must request an immediate guest frame for the fresh target"
    );
}

// ── needs_render / poll_inputs dirty gating ────────────────────────

#[test]
fn off_screen_dirty_surface_is_held_back_and_does_not_busy_wake() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);
    let now = Instant::now();

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Entering);
    assert!(
        slot.needs_render(now),
        "a fresh render target must paint its warm-up frame even off-screen"
    );

    // What render() records on commit; rendering for real needs EGL.
    slot.rendered_since_acquire = true;
    let _ = slot.surface.take_render_requested();

    slot.surface.mark_needs_render();
    assert!(
        !slot.needs_render(now),
        "a dirty surface in Entering with a committed buffer must not render — \
         renders punching through the swipe inhibition is the BDK-658 bug"
    );
    assert!(
        !slot.poll_inputs(now).surface_needs_render,
        "the held-back dirty flag must not wake the poll loop for renders \
         needs_render will refuse"
    );
    assert!(
        slot.surface.needs_render(),
        "holding back must retain the flag; the deferred render happens later"
    );

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Visible);
    assert!(
        slot.needs_render(now),
        "the transition to Visible must release the held-back dirty render"
    );
}

#[test]
fn transition_incoming_forces_the_pre_transition_frame_through_the_gate() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);
    let now = Instant::now();

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Entering);
    slot.rendered_since_acquire = true;
    let _ = slot.surface.take_render_requested();
    assert!(!slot.needs_render(now));

    dispatch_event(&mut slot, WidgetEvent::TransitionIncoming);
    assert!(
        slot.needs_render(now),
        "the compositor-demanded pre-transition frame must pass the gate; \
         scene cycling waits for it before starting the slide"
    );
}

#[test]
fn reacquiring_a_render_target_resets_the_warm_up() {
    let wat = hookless_widget_wat();
    let mut slot = test_slot(&wat);

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Entering);
    slot.rendered_since_acquire = true;

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Dormant);
    assert!(
        slot.render_target.is_none(),
        "Dormant must release the render target"
    );

    enter_state(&mut slot, bmc_widget_protocol::LifecycleState::Entering);
    assert!(
        slot.render_target.is_some(),
        "re-entering must acquire a fresh render target"
    );
    assert!(
        !slot.rendered_since_acquire,
        "a fresh target must get its unconditional warm-up frame; a stale \
         committed-buffer flag would leave the slot presenting nothing"
    );
    assert!(
        slot.needs_render(Instant::now()),
        "the warm-up render must be allowed immediately after acquisition"
    );
}
