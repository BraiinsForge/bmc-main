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

//! Host-to-guest event delivery and fixture replay helpers.

#![expect(clippy::too_many_lines)]

use anyhow::Result;
use bmc_wasm_protocol::{
    BitmapId, FetchOutcome, FetchRequestId, HttpListenerId, HttpRequestId, MdnsBrowseId, SocketId,
    SsdpSearchId, UdpBroadcastId, WebsocketId,
};
use std::ptr::NonNull;
use std::time::Duration;

use bmc_render::renderer::{AssetSuspendResult, AssetTagState, Renderer};

#[cfg(feature = "testing")]
use crate::host_api::CapturedMdnsEvent;
use crate::host_api::{
    CompletedFetch, FetchCompletionContext, FetchRequestKey, FixtureEvent, FixtureEventKind,
    HttpInboundRequest, MdnsEvent, SocketEvent, SsdpEvent, UdpBroadcastEvent, WsEvent,
};

use super::FetchLogDecision;
use super::backend::WasmWidgetRuntime;
use super::background::{Redirects, do_fetch};
use super::memory::alloc_and_copy_to_guest;

type DelayedFetchRequest = (
    String,
    String,
    Vec<(String, String)>,
    Option<Vec<u8>>,
    Duration,
    FetchRequestId,
);

impl WasmWidgetRuntime {
    /// Take all recorded events (drains the buffer). Used by the capture binary
    /// to write the combined fixture file after the capture loop finishes.
    pub fn take_recorded_events(&mut self) -> Vec<FixtureEvent> {
        std::mem::take(&mut self.store.data_mut().recorded_events)
    }

    /// Inject fixture events whose `at_ms` <= `monotonic_ms` into stub channels.
    ///
    /// Must be called each frame before `deliver_*` methods. Events are injected
    /// in order; the cursor advances so each event fires exactly once.
    pub fn inject_fixture_events(&mut self, monotonic_ms: u64) {
        let state = self.store.data_mut();
        let Some(ref mut ef) = state.event_fixtures else {
            return;
        };

        while ef.cursor < ef.events.len() && ef.events[ef.cursor].at_ms <= monotonic_ms {
            let event = &ef.events[ef.cursor];

            let delivered = match &event.kind {
                FixtureEventKind::WsOpen { ws_id } => {
                    if let Some(tx) = ef.ws_event_txs.get(ws_id) {
                        let _ = tx.send(WsEvent::Open);
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::WsMessage { ws_id, data } => {
                    if let Some(tx) = ef.ws_event_txs.get(ws_id) {
                        let _ = tx.send(WsEvent::Message(data.clone()));
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::WsClose { ws_id, code } => {
                    if let Some(tx) = ef.ws_event_txs.get(ws_id) {
                        let _ = tx.send(WsEvent::Close(*code));
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::SocketConnected { socket_id } => {
                    if let Some(tx) = ef.socket_event_txs.get(socket_id) {
                        let _ = tx.send(SocketEvent::Connected);
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::SocketData { socket_id, data } => {
                    if let Some(tx) = ef.socket_event_txs.get(socket_id) {
                        let _ = tx.send(SocketEvent::Data(data.clone()));
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::SocketClosed { socket_id, code } => {
                    if let Some(tx) = ef.socket_event_txs.get(socket_id) {
                        let _ = tx.send(SocketEvent::Closed(*code));
                        true
                    } else {
                        false
                    }
                }
                FixtureEventKind::SsdpFound { search_id, data } => {
                    if let Some(tx) = ef.ssdp_event_txs.get(search_id) {
                        let _ = tx.send(SsdpEvent::Found(data.clone()));
                    }
                    true
                }
                FixtureEventKind::SsdpRemoved { search_id, data } => {
                    if let Some(tx) = ef.ssdp_event_txs.get(search_id) {
                        let _ = tx.send(SsdpEvent::Removed(data.clone()));
                    }
                    true
                }
                FixtureEventKind::MdnsFound { browse_id, data } => {
                    if let Some(tx) = ef.mdns_event_txs.get(browse_id) {
                        let _ = tx.send(MdnsEvent::Found(data.clone()));
                    }
                    true
                }
                FixtureEventKind::MdnsRemoved { browse_id, data } => {
                    if let Some(tx) = ef.mdns_event_txs.get(browse_id) {
                        let _ = tx.send(MdnsEvent::Removed(data.clone()));
                    }
                    true
                }
                FixtureEventKind::UdpResponse {
                    broadcast_id,
                    data,
                    source,
                } => {
                    if let Some(tx) = ef.udp_event_txs.get(broadcast_id) {
                        let _ = tx.send(UdpBroadcastEvent::Response(data.clone(), source.clone()));
                    }
                    true
                }
                // Audio and LED events are informational — no-op during replay.
                FixtureEventKind::AudioPlay { .. }
                | FixtureEventKind::LedSetEndless { .. }
                | FixtureEventKind::LedSetTemporary { .. }
                | FixtureEventKind::LedStop => true,
            };

            if !delivered {
                tracing::debug!(
                    cursor = ef.cursor,
                    at_ms = event.at_ms,
                    kind = ?event.kind,
                    "fixture event deferred — channel not yet registered"
                );
                break;
            }
            ef.cursor += 1;
        }
    }

    /// Whether all fixture events up to the current virtual time have been injected.
    ///
    /// Returns `true` when there are no fixtures loaded, or when the cursor has
    /// advanced past all events whose `at_ms <= monotonic_ms`. Used by interaction
    /// settlement to avoid waiting for persistent browse/search sessions to close.
    #[must_use]
    pub fn fixture_events_caught_up(&self) -> bool {
        let state = self.store.data();
        let Some(ref ef) = state.event_fixtures else {
            return true;
        };
        ef.cursor >= ef.events.len() || ef.events[ef.cursor].at_ms > state.monotonic_ms
    }

    fn stage_fetch_responses(&mut self) {
        self.fire_ready_delayed_fetches();

        let state = self.store.data_mut();
        let responses = state.fetches.drain_settled();
        #[cfg(feature = "testing")]
        {
            state.delivered_events += responses.len() as u64;
        }

        state
            .staged_guest_deliveries
            .fetch_responses
            .extend(responses);
    }

    /// Check for completed fetch responses and delayed fetches, then deliver
    /// them to WASM by calling `__on_fetch_response`.
    ///
    /// Call this before `render()` each frame.
    pub fn deliver_fetch_responses(&mut self) {
        self.stage_fetch_responses();
        self.deliver_staged_fetch_responses();
    }

    fn deliver_staged_fetch_responses(&mut self) -> bool {
        let responses = std::mem::take(
            &mut self
                .store
                .data_mut()
                .staged_guest_deliveries
                .fetch_responses,
        );
        if responses.is_empty() {
            return false;
        }

        {
            let state = self.store.data_mut();
            for resp in &responses {
                let Some(key) = state.fetch_keys.remove(&resp.request_id) else {
                    continue;
                };
                if let Some(ref observer) = state.fetch_observer {
                    observer(&key.joined(), resp.status, &resp.body);
                }
                #[cfg(feature = "testing")]
                {
                    state.fetch_log_probe.last_refusal = match &resp.context {
                        FetchCompletionContext::CredentialRefusal(refusal) => {
                            Some(refusal.to_string())
                        }
                        FetchCompletionContext::Normal
                        | FetchCompletionContext::HermeticRefusal => None,
                    };
                }

                match state.fetch_log_limiter.record(
                    &key,
                    resp.status,
                    &resp.context,
                    state.monotonic_ms,
                ) {
                    FetchLogDecision::LogSuccess => {
                        tracing::debug!(
                            request_id = resp.request_id.to_wire(),
                            method = %key.shown_method(),
                            url = %key.shown_url(),
                            status = resp.status,
                            outcome = ?FetchOutcome::from_wire(resp.status),
                            body_len = resp.body.len(),
                            "fetch succeeded"
                        );
                    }
                    FetchLogDecision::LogFailure {
                        admission,
                        previous_status,
                    } => {
                        #[cfg(feature = "testing")]
                        {
                            state.fetch_log_probe.failure_log_count += 1;
                        }
                        match &resp.context {
                            FetchCompletionContext::CredentialRefusal(refusal) => {
                                tracing::warn!(
                                    request_id = resp.request_id.to_wire(),
                                    method = %key.shown_method(),
                                    url = %key.shown_url(),
                                    status = resp.status,
                                    outcome = ?FetchOutcome::from_wire(resp.status),
                                    body_len = resp.body.len(),
                                    admission = admission.as_str(),
                                    refusal = %refusal,
                                    previous_status = ?previous_status,
                                    "refusing fetch: {refusal}"
                                );
                            }
                            FetchCompletionContext::Normal
                            | FetchCompletionContext::HermeticRefusal => {
                                tracing::warn!(
                                    request_id = resp.request_id.to_wire(),
                                    method = %key.shown_method(),
                                    url = %key.shown_url(),
                                    status = resp.status,
                                    outcome = ?FetchOutcome::from_wire(resp.status),
                                    body_len = resp.body.len(),
                                    admission = admission.as_str(),
                                    previous_status = ?previous_status,
                                    "fetch failed"
                                );
                            }
                        }
                    }
                    FetchLogDecision::NoLog => {}
                }
            }
        }

        tracing::debug!("delivering {} fetch response(s)", responses.len());

        let on_response = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_fetch_response");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_response), Ok(alloc_func)) = (on_response, alloc_func) else {
            tracing::warn!("widget missing __on_fetch_response or __alloc export");
            return false;
        };

        for resp in responses {
            let Some((body_ptr, body_len)) =
                self.alloc_guest_bytes(alloc_func, &resp.body, "fetch response body")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_response.call(
                &mut self.store,
                (resp.request_id.to_wire(), resp.status, body_ptr, body_len),
            ) {
                self.record_guest_trap("__on_fetch_response", &e);
                break;
            }
        }
        true
    }

    fn stage_image_decode_results(&mut self) {
        let state = self.store.data_mut();
        while let Ok(done) = state.image_decode_rx.try_recv() {
            state.in_flight_image_decodes = state.in_flight_image_decodes.saturating_sub(1);
            state.staged_guest_deliveries.image_decodes.push(done);
        }
    }

    /// Register completed off-thread image decodes and notify the guest via
    /// `__on_image_ready`. Dormant cache-backed pixels upload when a draw uses them.
    /// A renderer-less poll stages completed work until a renderer is available.
    pub fn deliver_image_decode_results(&mut self) {
        self.stage_image_decode_results();
        self.deliver_staged_image_decode_results();
    }

    fn deliver_staged_image_decode_results(&mut self) -> bool {
        let state = self.store.data_mut();
        if state.renderer_ptr.is_none() || state.staged_guest_deliveries.image_decodes.is_empty() {
            return false;
        }
        let completed = std::mem::take(&mut state.staged_guest_deliveries.image_decodes);

        let Ok(on_ready) = self
            .instance
            .get_typed_func::<(u32, u32), ()>(&self.store, "__on_image_ready")
        else {
            // Widget did not opt into async image decode; drop the results.
            return false;
        };
        // Fired while dormant to reclaim the guest's pending entry (optional export).
        let on_dropped = self
            .instance
            .get_typed_func::<u32, ()>(&self.store, "__on_image_dropped")
            .ok();

        let dormant = self.store.data().renderer_assets_are_dormant();
        for done in completed {
            self.store
                .data_mut()
                .add_profile_us("image_decode_us", done.decode_us);
            // A `0` bitmap id is the absent sentinel — the guest reads it as a
            // decode failure.
            let backing = match done.cache_write {
                crate::host_api::CacheWriteOutcome::Stored => {
                    crate::renderer_assets::AssetBacking::Cache(done.raw_tag.clone())
                }
                crate::host_api::CacheWriteOutcome::Failed(error) => {
                    tracing::warn!(tag = %done.raw_tag, %error, "image cache write failed");
                    crate::renderer_assets::AssetBacking::Volatile
                }
                crate::host_api::CacheWriteOutcome::Disabled => {
                    crate::renderer_assets::AssetBacking::Volatile
                }
            };
            let cache_backed = matches!(&backing, crate::renderer_assets::AssetBacking::Cache(_));
            let lazy = dormant && cache_backed;
            let skip_dormant_upload = dormant && !cache_backed;
            let bitmap_id = match done.result {
                Ok(_) if skip_dormant_upload => 0,
                Ok(decoded) => decoded.consume_with_rgba(|rgba, width, height| {
                    let started = std::time::Instant::now();
                    let id = self
                        .register_decoded_bitmap(
                            &done.raw_tag,
                            &done.tag,
                            rgba,
                            width,
                            height,
                            backing,
                        )
                        .map_or(0, BitmapId::to_ffi);
                    let upload_us =
                        u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                    if !lazy {
                        self.store
                            .data_mut()
                            .add_profile_us("image_upload_us", upload_us);
                    }
                    id
                }),
                Err(e) => {
                    tracing::error!(job = done.job_id.to_wire(), "image decode failed: {e}");
                    0
                }
            };
            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            // Dormant: reclaim the pending entry but don't drive the widget.
            if dormant {
                if let Some(on_dropped) = on_dropped
                    && let Err(e) = on_dropped.call(&mut self.store, done.job_id.to_wire())
                {
                    self.record_guest_trap("__on_image_dropped", &e);
                    break;
                }
                continue;
            }
            if let Err(e) = on_ready.call(&mut self.store, (done.job_id.to_wire(), bitmap_id)) {
                self.record_guest_trap("__on_image_ready", &e);
                break;
            }
        }
        true
    }

    /// Reborrow the parked renderer to upload a pre-decoded RGBA buffer.
    fn register_decoded_bitmap(
        &mut self,
        raw_tag: &str,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
        backing: crate::renderer_assets::AssetBacking,
    ) -> Option<BitmapId> {
        let kind = crate::renderer_assets::RendererAssetKind::Bitmap(
            bmc_wasm_protocol::BitmapSampling::Linear,
        );
        if !self
            .store
            .data()
            .renderer_asset_registration_matches(raw_tag, kind, &backing)
        {
            return None;
        }
        let mut ptr = self.store.data().renderer_ptr?;
        // SAFETY: parked by `WasmWidgetRuntime::with_renderer` on this thread;
        // single-threaded wasmi dispatch means no other `&mut Renderer` is live.
        let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
        let cache_backed = matches!(backing, crate::renderer_assets::AssetBacking::Cache(_));
        let lazy = cache_backed && self.store.data().renderer_assets_are_dormant();
        let id = if lazy {
            let id = renderer.reserve_bitmap(tag)?;
            match renderer.bitmap_tag_state(tag) {
                AssetTagState::Resident(resident) if resident == id => {
                    self.store.data_mut().require_renderer_gpu_access().ok()?;
                    if !matches!(
                        renderer.suspend_bitmap(tag),
                        AssetSuspendResult::Suspended(suspended) if suspended == id
                    ) {
                        return None;
                    }
                }
                AssetTagState::Suspended(suspended) if suspended == id => {}
                AssetTagState::Resident(_)
                | AssetTagState::Suspended(_)
                | AssetTagState::Unknown => {
                    return None;
                }
            }
            Some(id)
        } else {
            self.store.data_mut().require_renderer_gpu_access().ok()?;
            renderer.register_bitmap_rgba(tag, rgba, width, height)
        }?;
        let asset_id = crate::renderer_assets::RendererAssetId::Bitmap(id);
        let recorded = self.store.data_mut().record_renderer_asset(
            raw_tag.to_owned(),
            kind,
            asset_id,
            backing,
        );
        if recorded && cache_backed {
            if lazy {
                self.store.data_mut().renderer_assets.mark_pending(raw_tag);
            } else {
                self.store.data_mut().renderer_assets.mark_resident(raw_tag);
            }
        }
        recorded.then_some(id)
    }

    /// Whether there are pending or in-flight fetches that need polling.
    #[must_use]
    pub fn has_pending_fetches(&self) -> bool {
        let state = self.store.data();
        state.fetches.has_pending()
    }

    /// Whether an accepted fetch is awaiting delivery to the widget.
    #[must_use]
    pub fn has_in_flight_fetches(&self) -> bool {
        self.store.data().fetches.has_in_flight()
    }

    fn stage_ws_messages(&mut self) {
        let mut events: Vec<(WebsocketId, WsEvent)> = Vec::new();
        let mut closed_ids: Vec<WebsocketId> = Vec::new();

        let state = self.store.data_mut();
        for (&ws_id, ws) in &state.websockets {
            while let Ok(event) = ws.event_rx.try_recv() {
                let is_close = matches!(event, WsEvent::Close(_));
                events.push((ws_id, event));
                if is_close {
                    closed_ids.push(ws_id);
                }
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }
        for id in &closed_ids {
            state.websockets.remove(id);
        }

        if state.record_events {
            let at_ms = state.monotonic_ms;
            for (ws_id, event) in &events {
                let kind = match event {
                    WsEvent::Open => FixtureEventKind::WsOpen { ws_id: *ws_id },
                    WsEvent::Message(data) => FixtureEventKind::WsMessage {
                        ws_id: *ws_id,
                        data: data.clone(),
                    },
                    WsEvent::Close(code) => FixtureEventKind::WsClose {
                        ws_id: *ws_id,
                        code: *code,
                    },
                };
                state.recorded_events.push(FixtureEvent { at_ms, kind });
            }
        }

        state
            .staged_guest_deliveries
            .websocket_events
            .extend(events);
    }

    /// Drain WebSocket events from all active connections and deliver them
    /// to WASM by calling `__on_ws_event(ws_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Open, 1 = Message, 2 = Close (data_ptr/data_len carry
    /// the close code as two little-endian bytes).
    pub fn deliver_ws_messages(&mut self) -> bool {
        self.stage_ws_messages();
        self.deliver_staged_ws_messages()
    }

    fn deliver_staged_ws_messages(&mut self) -> bool {
        let events = std::mem::take(
            &mut self
                .store
                .data_mut()
                .staged_guest_deliveries
                .websocket_events,
        );

        if events.is_empty() {
            return false;
        }

        tracing::debug!("delivering {} WS event(s)", events.len());

        let on_ws_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_ws_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_ws_event), Ok(alloc_func)) = (on_ws_event, alloc_func) else {
            tracing::warn!("widget missing __on_ws_event or __alloc export");
            return false;
        };

        for (ws_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                WsEvent::Open => (0, &[]),
                WsEvent::Message(bytes) => (1, bytes),
                WsEvent::Close(code) => (2, &code.to_le_bytes()),
            };

            let Some((data_ptr, data_len)) =
                self.alloc_guest_bytes(alloc_func, data, "websocket event payload")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_ws_event.call(
                &mut self.store,
                (ws_id.to_wire(), event_type, data_ptr, data_len),
            ) {
                self.record_guest_trap("__on_ws_event", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active WebSocket connections.
    #[must_use]
    pub fn has_active_websockets(&self) -> bool {
        !self.store.data().websockets.is_empty()
    }

    fn stage_socket_events(&mut self) {
        let mut events: Vec<(SocketId, SocketEvent)> = Vec::new();
        let mut closed_ids: Vec<SocketId> = Vec::new();

        let state = self.store.data_mut();
        for (&socket_id, sock) in &state.sockets {
            while let Ok(event) = sock.event_rx.try_recv() {
                let is_close = matches!(event, SocketEvent::Closed(_));
                events.push((socket_id, event));
                if is_close {
                    closed_ids.push(socket_id);
                }
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }
        for id in &closed_ids {
            state.sockets.remove(id);
        }

        if state.record_events {
            let at_ms = state.monotonic_ms;
            for (socket_id, event) in &events {
                let kind = match event {
                    SocketEvent::Connected => FixtureEventKind::SocketConnected {
                        socket_id: *socket_id,
                    },
                    SocketEvent::Data(data) => FixtureEventKind::SocketData {
                        socket_id: *socket_id,
                        data: data.clone(),
                    },
                    SocketEvent::Closed(code) => FixtureEventKind::SocketClosed {
                        socket_id: *socket_id,
                        code: *code,
                    },
                };
                state.recorded_events.push(FixtureEvent { at_ms, kind });
            }
        }

        state.staged_guest_deliveries.socket_events.extend(events);
    }

    /// Drain socket events from all active connections and deliver them
    /// to WASM by calling `__on_socket_event(socket_id, event_type, data_ptr, data_len)`.
    ///
    /// Event types: 0 = Connected, 1 = Data, 2 = Closed.
    pub fn deliver_socket_events(&mut self) -> bool {
        self.stage_socket_events();
        self.deliver_staged_socket_events()
    }

    fn deliver_staged_socket_events(&mut self) -> bool {
        let events =
            std::mem::take(&mut self.store.data_mut().staged_guest_deliveries.socket_events);

        if events.is_empty() {
            return false;
        }

        let on_socket_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_socket_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_socket_event), Ok(alloc_func)) = (on_socket_event, alloc_func) else {
            tracing::warn!("widget missing __on_socket_event or __alloc export");
            return false;
        };

        for (socket_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                SocketEvent::Connected => (0, &[]),
                SocketEvent::Data(bytes) => (1, bytes),
                SocketEvent::Closed(code) => (2, &code.to_le_bytes()),
            };

            let Some((data_ptr, data_len)) =
                self.alloc_guest_bytes(alloc_func, data, "socket event payload")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_socket_event.call(
                &mut self.store,
                (socket_id.to_wire(), event_type, data_ptr, data_len),
            ) {
                self.record_guest_trap("__on_socket_event", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active socket connections.
    #[must_use]
    pub fn has_active_sockets(&self) -> bool {
        !self.store.data().sockets.is_empty()
    }

    fn stage_mdns_events(&mut self) {
        let mut events: Vec<(MdnsBrowseId, MdnsEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&browse_id, browse) in &state.mdns_browses {
            while let Ok(event) = browse.event_rx.try_recv() {
                events.push((browse_id, event));
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }

        if state.record_events {
            let at_ms = state.monotonic_ms;
            for (browse_id, event) in &events {
                let kind = match event {
                    MdnsEvent::Found(data) => FixtureEventKind::MdnsFound {
                        browse_id: *browse_id,
                        data: data.clone(),
                    },
                    MdnsEvent::Removed(data) => FixtureEventKind::MdnsRemoved {
                        browse_id: *browse_id,
                        data: data.clone(),
                    },
                };
                state.recorded_events.push(FixtureEvent { at_ms, kind });
            }
        }

        state.staged_guest_deliveries.mdns_events.extend(events);
    }

    /// Drain mDNS events from all active browse sessions and deliver them
    /// to WASM by calling `__on_mdns_event(browse_id, event_type, data_ptr, data_len)`.
    pub fn deliver_mdns_events(&mut self) -> bool {
        self.stage_mdns_events();
        self.deliver_staged_mdns_events()
    }

    fn deliver_staged_mdns_events(&mut self) -> bool {
        let events = std::mem::take(&mut self.store.data_mut().staged_guest_deliveries.mdns_events);

        if events.is_empty() {
            return false;
        }

        #[cfg(feature = "testing")]
        {
            let state = self.store.data_mut();
            for (_, event) in &events {
                if let MdnsEvent::Found(json) = event {
                    let fullname = serde_json::from_str::<serde_json::Value>(json)
                        .ok()
                        .and_then(|v| v["name"].as_str().map(str::to_owned))
                        .unwrap_or_default();
                    state
                        .mdns_captured_events
                        .push(CapturedMdnsEvent { fullname });
                }
            }
        }

        let on_mdns_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_mdns_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_mdns_event), Ok(alloc_func)) = (on_mdns_event, alloc_func) else {
            tracing::warn!("widget missing __on_mdns_event or __alloc export");
            return false;
        };

        for (browse_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                MdnsEvent::Found(json) => (0, json.as_bytes()),
                MdnsEvent::Removed(name) => (1, name.as_bytes()),
            };

            let Some((data_ptr, data_len)) =
                self.alloc_guest_bytes(alloc_func, data, "mDNS event payload")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_mdns_event.call(
                &mut self.store,
                (browse_id.to_wire(), event_type, data_ptr, data_len),
            ) {
                self.record_guest_trap("__on_mdns_event", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active mDNS browse sessions.
    #[must_use]
    pub fn has_active_mdns_browses(&self) -> bool {
        !self.store.data().mdns_browses.is_empty()
    }

    fn stage_ssdp_events(&mut self) {
        let mut events: Vec<(SsdpSearchId, SsdpEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&search_id, search) in &state.ssdp_searches {
            while let Ok(event) = search.event_rx.try_recv() {
                events.push((search_id, event));
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }

        if state.record_events {
            let at_ms = state.monotonic_ms;
            for (search_id, event) in &events {
                let kind = match event {
                    SsdpEvent::Found(data) => FixtureEventKind::SsdpFound {
                        search_id: *search_id,
                        data: data.clone(),
                    },
                    SsdpEvent::Removed(data) => FixtureEventKind::SsdpRemoved {
                        search_id: *search_id,
                        data: data.clone(),
                    },
                };
                state.recorded_events.push(FixtureEvent { at_ms, kind });
            }
        }

        state.staged_guest_deliveries.ssdp_events.extend(events);
    }

    /// Drain SSDP events from all active search sessions and deliver them
    /// to WASM by calling `__on_ssdp_event(search_id, event_type, data_ptr, data_len)`.
    pub fn deliver_ssdp_events(&mut self) -> bool {
        self.stage_ssdp_events();
        self.deliver_staged_ssdp_events()
    }

    fn deliver_staged_ssdp_events(&mut self) -> bool {
        let events = std::mem::take(&mut self.store.data_mut().staged_guest_deliveries.ssdp_events);

        if events.is_empty() {
            return false;
        }

        let on_ssdp_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32), ()>(&self.store, "__on_ssdp_event");
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_ssdp_event), Ok(alloc_func)) = (on_ssdp_event, alloc_func) else {
            tracing::warn!("widget missing __on_ssdp_event or __alloc export");
            return false;
        };

        for (search_id, event) in events {
            let (event_type, data): (u32, &[u8]) = match &event {
                SsdpEvent::Found(json) => (0, json.as_bytes()),
                SsdpEvent::Removed(usn) => (1, usn.as_bytes()),
            };

            let Some((data_ptr, data_len)) =
                self.alloc_guest_bytes(alloc_func, data, "SSDP event payload")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_ssdp_event.call(
                &mut self.store,
                (search_id.to_wire(), event_type, data_ptr, data_len),
            ) {
                self.record_guest_trap("__on_ssdp_event", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active SSDP search sessions.
    #[must_use]
    pub fn has_active_ssdp_searches(&self) -> bool {
        !self.store.data().ssdp_searches.is_empty()
    }

    fn stage_udp_broadcast_events(&mut self) {
        let mut events: Vec<(UdpBroadcastId, UdpBroadcastEvent)> = Vec::new();

        let state = self.store.data_mut();
        for (&broadcast_id, broadcast) in &state.udp_broadcasts {
            while let Ok(event) = broadcast.event_rx.try_recv() {
                events.push((broadcast_id, event));
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }

        if state.record_events {
            let at_ms = state.monotonic_ms;
            for (broadcast_id, event) in &events {
                let UdpBroadcastEvent::Response(data, source) = event;
                state.recorded_events.push(FixtureEvent {
                    at_ms,
                    kind: FixtureEventKind::UdpResponse {
                        broadcast_id: *broadcast_id,
                        data: data.clone(),
                        source: source.clone(),
                    },
                });
            }
        }

        state.staged_guest_deliveries.udp_events.extend(events);
    }

    /// Drain UDP broadcast events from all active sessions and deliver them
    /// to WASM by calling `__on_udp_broadcast_event`.
    pub fn deliver_udp_broadcast_events(&mut self) -> bool {
        self.stage_udp_broadcast_events();
        self.deliver_staged_udp_broadcast_events()
    }

    fn deliver_staged_udp_broadcast_events(&mut self) -> bool {
        let events = std::mem::take(&mut self.store.data_mut().staged_guest_deliveries.udp_events);

        if events.is_empty() {
            return false;
        }

        let on_udp_broadcast_event = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32, u32), ()>(
                &self.store,
                "__on_udp_broadcast_event",
            );
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_udp_broadcast_event), Ok(alloc_func)) = (on_udp_broadcast_event, alloc_func)
        else {
            tracing::warn!("widget missing __on_udp_broadcast_event or __alloc export");
            return false;
        };

        for (broadcast_id, event) in events {
            let UdpBroadcastEvent::Response(ref data, ref source) = event;

            let Some((data_ptr, data_len)) =
                self.alloc_guest_bytes(alloc_func, data.as_bytes(), "UDP broadcast payload")
            else {
                continue;
            };
            let Some((source_ptr, source_len)) =
                self.alloc_guest_bytes(alloc_func, source.as_bytes(), "UDP broadcast source")
            else {
                continue;
            };

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_udp_broadcast_event.call(
                &mut self.store,
                (
                    broadcast_id.to_wire(),
                    data_ptr,
                    data_len,
                    source_ptr,
                    source_len,
                ),
            ) {
                self.record_guest_trap("__on_udp_broadcast_event", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active UDP broadcast sessions.
    #[must_use]
    pub fn has_active_udp_broadcasts(&self) -> bool {
        !self.store.data().udp_broadcasts.is_empty()
    }

    fn stage_http_requests(&mut self) {
        let mut requests: Vec<(HttpListenerId, HttpInboundRequest)> = Vec::new();

        let state = self.store.data_mut();
        for (&listener_id, listener) in &state.http_listeners {
            while let Ok(req) = listener.request_rx.try_recv() {
                requests.push((listener_id, req));
                #[cfg(feature = "testing")]
                {
                    state.delivered_events += 1;
                }
            }
        }

        state.staged_guest_deliveries.http_requests.extend(requests);
    }

    /// Drain inbound HTTP requests from all active listeners and deliver them
    /// to WASM by calling `__on_http_request(...)`.
    pub fn deliver_http_requests(&mut self) -> bool {
        self.stage_http_requests();
        self.deliver_staged_http_requests()
    }

    fn deliver_staged_http_requests(&mut self) -> bool {
        let requests =
            std::mem::take(&mut self.store.data_mut().staged_guest_deliveries.http_requests);

        if requests.is_empty() {
            return false;
        }

        let on_http_request = self
            .instance
            .get_typed_func::<(u32, u32, u32, u32, u32, u32, u32, u32, u32, u32), ()>(
                &self.store,
                "__on_http_request",
            );
        let alloc_func = self
            .instance
            .get_typed_func::<u32, u32>(&self.store, "__alloc");

        let (Ok(on_http_request), Ok(alloc_func)) = (on_http_request, alloc_func) else {
            tracing::warn!("widget missing __on_http_request or __alloc export");
            return false;
        };

        for (listener_id, req) in requests {
            let state = self.store.data_mut();
            let request_id = HttpRequestId::alloc(&mut state.next_http_request_id);
            state.http_response_txs.insert(request_id, req.response_tx);

            let (method_ptr, method_len) = self
                .alloc_guest_bytes(alloc_func, req.method.as_bytes(), "HTTP request method")
                .unwrap_or((0, 0));
            let (path_ptr, path_len) = self
                .alloc_guest_bytes(alloc_func, req.path.as_bytes(), "HTTP request path")
                .unwrap_or((0, 0));
            let (headers_ptr, headers_len) = self
                .alloc_guest_bytes(alloc_func, req.headers.as_bytes(), "HTTP request headers")
                .unwrap_or((0, 0));
            let (body_ptr, body_len) = self
                .alloc_guest_bytes(alloc_func, &req.body, "HTTP request body")
                .unwrap_or((0, 0));

            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::error!("set_fuel failed: {e}");
                continue;
            }
            if let Err(e) = on_http_request.call(
                &mut self.store,
                (
                    listener_id.to_wire(),
                    request_id.to_wire(),
                    method_ptr,
                    method_len,
                    path_ptr,
                    path_len,
                    headers_ptr,
                    headers_len,
                    body_ptr,
                    body_len,
                ),
            ) {
                self.record_guest_trap("__on_http_request", &e);
                break;
            }
        }
        true
    }

    /// Whether there are any active HTTP listeners.
    #[must_use]
    pub fn has_active_http_listeners(&self) -> bool {
        !self.store.data().http_listeners.is_empty()
    }

    /// Record a trap taken by a guest delivery callback.
    ///
    /// Keeps the first one: later callbacks in the same pass run on an instance
    /// that is already damaged, so their traps say nothing new.
    pub(super) fn record_guest_trap(&mut self, export: &str, error: &wasmi::Error) {
        tracing::error!(
            instance_id = %self.store.data().instance_id,
            export,
            trap = %error,
            "widget delivery callback trapped; tearing down"
        );
        if self.guest_trap.is_none() {
            self.guest_trap = Some(anyhow::anyhow!("{export} trapped: {error}"));
        }
    }

    fn take_guest_trap(&mut self) -> Result<()> {
        match self.guest_trap.take() {
            Some(trap) => Err(trap),
            None => Ok(()),
        }
    }

    /// Drain host channels without invoking the guest or touching the renderer.
    pub fn stage_deliveries(&mut self) {
        self.stage_fetch_responses();
        self.stage_image_decode_results();
        self.stage_ws_messages();
        self.stage_socket_events();
        self.stage_mdns_events();
        self.stage_ssdp_events();
        self.stage_udp_broadcast_events();
        self.stage_http_requests();
    }

    /// Whether a renderer-backed delivery scope has concrete work to run.
    #[must_use]
    pub fn has_staged_renderer_delivery(&self) -> bool {
        self.has_pending_lifecycle() || !self.store.data().staged_guest_deliveries.is_empty()
    }

    /// Drive every pending lifecycle hook and staged delivery in a fixed order.
    ///
    /// The multi-slot host calls this once per slot in every main-loop iteration.
    /// The ordering below is the canonical sequence guests observe; these calls
    /// must stay in this order so payload ordering remains stable across host
    /// revisions. Processing stops at the first guest trap.
    ///
    /// # Errors
    ///
    /// Returns the trap a delivery callback took. The instance cannot be
    /// recovered afterwards and the caller must tear it down.
    fn deliver_staged(&mut self) -> Result<()> {
        self.flush_pending_lifecycle();
        self.take_guest_trap()?;
        self.deliver_staged_fetch_responses();
        self.take_guest_trap()?;
        self.deliver_staged_image_decode_results();
        self.take_guest_trap()?;
        self.deliver_staged_ws_messages();
        self.take_guest_trap()?;
        self.deliver_staged_socket_events();
        self.take_guest_trap()?;
        self.deliver_staged_mdns_events();
        self.take_guest_trap()?;
        self.deliver_staged_ssdp_events();
        self.take_guest_trap()?;
        self.deliver_staged_udp_broadcast_events();
        self.take_guest_trap()?;
        self.deliver_staged_http_requests();
        self.take_guest_trap()
    }

    /// Stage and deliver all pending work.
    ///
    /// # Errors
    ///
    /// Returns the first trap taken by a delivery callback.
    pub fn poll_deliveries(&mut self) -> Result<()> {
        self.stage_deliveries();
        self.deliver_staged()
    }

    /// Fan out to every `deliver_*` entry point while renderer-backed imports
    /// are available to guest callbacks.
    ///
    /// Dynamic assets can arrive from async callbacks rather than `render`
    /// itself. For example, media-control receives MPD album art on a socket
    /// callback and registers it as a bitmap before requesting the next frame.
    /// The caller-owned renderer therefore has to be parked while delivery
    /// callbacks run, not just while `render()` runs.
    /// Returns whether delivery accessed the renderer and requires a GPU fence.
    ///
    /// # Errors
    ///
    /// Propagates a delivery callback's trap; see [`Self::poll_deliveries`].
    pub fn poll_deliveries_with_renderer(
        &mut self,
        renderer: NonNull<dyn Renderer>,
    ) -> Result<bool> {
        self.stage_deliveries();
        self.poll_staged_deliveries_with_renderer(renderer)
    }

    /// Stage and deliver work while acquiring host GPU access on first mutation.
    pub fn poll_deliveries_with_renderer_and_gpu_access<F>(
        &mut self,
        renderer: NonNull<dyn Renderer>,
        require_gpu_access: F,
    ) -> Result<bool>
    where
        F: FnMut() -> anyhow::Result<()>,
    {
        self.stage_deliveries();
        self.poll_staged_deliveries_with_renderer_and_gpu_access(renderer, require_gpu_access)
    }

    /// Deliver staged work and return whether it accessed GPU resources.
    ///
    /// # Errors
    ///
    /// Propagates a staged delivery callback's trap.
    pub fn poll_staged_deliveries_with_renderer(
        &mut self,
        renderer: NonNull<dyn Renderer>,
    ) -> Result<bool> {
        self.poll_staged_deliveries_with_renderer_and_gpu_access(renderer, || Ok(()))
    }

    /// Deliver staged work, acquiring host GPU access only if delivery mutates GPU resources.
    ///
    /// # Errors
    ///
    /// Propagates a staged callback trap or GPU-access acquisition failure.
    pub fn poll_staged_deliveries_with_renderer_and_gpu_access<F>(
        &mut self,
        renderer: NonNull<dyn Renderer>,
        mut require_gpu_access: F,
    ) -> Result<bool>
    where
        F: FnMut() -> anyhow::Result<()>,
    {
        self.store.data_mut().begin_renderer_delivery();
        let result = self.with_renderer_gpu_access(&mut require_gpu_access, |runtime| {
            runtime.with_renderer(renderer, Self::deliver_staged)
        });
        let gpu_access_failure = self.store.data_mut().take_renderer_gpu_access_failure();
        result?;
        if let Some(error) = gpu_access_failure {
            anyhow::bail!("renderer GPU access failed: {error}");
        }
        Ok(self.store.data().renderer_was_accessed_during_delivery())
    }

    pub(super) fn with_renderer_gpu_access<F, R>(
        &mut self,
        require_gpu_access: &mut F,
        operation: impl FnOnce(&mut Self) -> R,
    ) -> R
    where
        F: FnMut() -> anyhow::Result<()>,
    {
        self.store
            .data_mut()
            .install_renderer_gpu_access(require_gpu_access);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));
        let state = self.store.data_mut();
        state.renderer_ptr = None;
        state.clear_renderer_gpu_access();
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// True while an async image decode is in flight or awaiting renderer delivery.
    #[must_use]
    pub fn has_pending_image_decodes(&self) -> bool {
        let state = self.store.data();
        state.in_flight_image_decodes > 0 || !state.staged_guest_deliveries.image_decodes.is_empty()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn has_completed_image_decodes_for_test(&self) -> bool {
        !self
            .store
            .data()
            .staged_guest_deliveries
            .image_decodes
            .is_empty()
    }

    /// Predicate consumed by the multi-slot host's `compute_poll_timeout` to clamp the
    /// `poll(2)` wakeup to 100 ms whenever any slot still has async work that could
    /// produce a delivery before the next render or lifecycle event.
    #[must_use]
    pub fn has_pending_io(&self) -> bool {
        self.has_pending_fetches()
            || self.has_pending_image_decodes()
            || self.has_active_websockets()
            || self.has_active_sockets()
            || self.has_active_mdns_browses()
            || self.has_active_ssdp_searches()
            || self.has_active_udp_broadcasts()
            || self.has_active_http_listeners()
    }

    fn alloc_guest_bytes(
        &mut self,
        alloc_func: wasmi::TypedFunc<u32, u32>,
        bytes: &[u8],
        context: &str,
    ) -> Option<(u32, u32)> {
        alloc_and_copy_to_guest(
            self.instance,
            &mut self.store,
            alloc_func,
            self.fuel_per_frame,
            bytes,
            context,
        )
    }

    fn fire_ready_delayed_fetches(&mut self) {
        let state = self.store.data_mut();
        let now_ms = state.monotonic_ms;
        let mut ready: Vec<DelayedFetchRequest> = Vec::new();
        state.fetches.delayed_mut().retain(|df| {
            if now_ms >= df.fire_at_ms {
                ready.push((
                    df.method.clone(),
                    df.url.clone(),
                    df.headers.clone(),
                    df.body.clone(),
                    df.timeout,
                    df.request_id,
                ));
                false
            } else {
                true
            }
        });

        for (method, url, headers, body, timeout, request_id) in ready {
            tracing::debug!(request_id = request_id.to_wire(), %method, %url, "firing HTTP fetch");
            let key = FetchRequestKey::new(&method, &url);
            state.fetch_keys.insert(request_id, key.clone());
            let settle = state.fetches.accept(request_id);

            let intercepted = state
                .fetch_interceptor
                .as_ref()
                .and_then(|f| f(&method, &url));
            if let Some((status, body)) = intercepted {
                let _ = settle.send(CompletedFetch {
                    request_id,
                    status,
                    body,
                    context: FetchCompletionContext::Normal,
                });
                continue;
            }

            if state.refuse_live_io("fetch", &key.joined()) {
                let _ = settle.send(CompletedFetch {
                    request_id,
                    status: FetchOutcome::Network.to_wire(),
                    body: Vec::new(),
                    context: FetchCompletionContext::HermeticRefusal,
                });
                continue;
            }

            // Resolved here rather than when the fetch was queued,
            // so a rotated secret is the one that goes out
            // and the queue never holds a secret at all.
            let spent = match super::imports::credentials::spend(state, &url, &headers, body) {
                Ok(spent) => spent,
                Err(refusal) => {
                    let _ = settle.send(CompletedFetch {
                        request_id,
                        status: FetchOutcome::Refused.to_wire(),
                        body: Vec::new(),
                        context: FetchCompletionContext::CredentialRefusal(refusal),
                    });
                    continue;
                }
            };
            let super::imports::credentials::SpentRequest {
                url: resolved,
                headers,
                body,
                carries_secret,
            } = spent;
            let redirects = Redirects::for_request(carries_secret);

            let tx = settle;
            let agent = state.fetch_agent.clone();
            std::thread::spawn(move || {
                let (status, resp_body) = do_fetch(
                    &agent,
                    &method,
                    &resolved,
                    &headers,
                    body.as_deref(),
                    timeout,
                    redirects,
                );
                let _ = tx.send(CompletedFetch {
                    request_id,
                    status,
                    body: resp_body,
                    context: FetchCompletionContext::Normal,
                });
            });
        }
    }
}
