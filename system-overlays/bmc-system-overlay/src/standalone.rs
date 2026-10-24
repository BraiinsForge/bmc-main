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

//! Standalone overlay process: own EGL/renderer/connection and a poll loop.

use std::time::{Duration, Instant};

use bmc_gpu_render_lock::GpuRenderLock;
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::FrameClear;
use bmc_render::renderer::prelude::*;
use bmc_widget::egl::{EglContext, SharedRenderScratch};

use crate::gpu::{OverlayRenderTarget, wait_for_gpu};
use crate::overlay::{
    AlarmEvent, FrameKind, LayerConfig, MIN_INTER_FRAME, OverlayFrames, RenderOutcome,
    SystemOverlay, deliver_localization, deliver_platform_events, deliver_upgrade_update_and_tick,
    resize_transition, resolved_configured_size, screen_edge_visible, whole_millis_up,
};
use crate::surface::LayerSurfaceClient;

/// Run an overlay as its own process: own connection, renderer, and loop.
#[expect(
    clippy::too_many_lines,
    reason = "single linear per-pass driver loop; each control protocol adds a \
              deliver/forward pair that reads clearest inline with the loop"
)]
pub fn run_standalone(mut overlay: Box<dyn SystemOverlay>) -> anyhow::Result<()> {
    let config: LayerConfig = overlay.layer_config();

    let mut client = LayerSurfaceClient::connect(
        &config,
        crate::surface::ProtocolOptIns::from_overlay(overlay.as_ref()),
    )?;
    let mut size = resolved_configured_size(config.size, client.size());

    let egl = EglContext::new()?;
    let mut scratch = SharedRenderScratch::new(&egl, size.0, size.1)?;
    let gpu_lock = GpuRenderLock::from_env()?;
    let mut renderer = create_standalone_renderer(&scratch, size)?;
    let mut target = OverlayRenderTarget::new(&egl, size.0, size.1)?;

    overlay.init();
    let screen_edge = overlay.screen_edge();
    if let Some(edge) = screen_edge {
        client.create_screen_edge(edge)?;
    }
    let mut revealed = false;
    let mut last_render: Option<Instant> = None;
    let mut mapped = false;
    let mut frames = OverlayFrames::new(overlay.animation_frame_delay_ms());

    while client.running() {
        // Drain what the previous poll_dispatch delivered.
        for ev in client.drain_touch() {
            overlay.on_touch(ev);
        }
        for released in client.drain_released_buffers() {
            target.mark_released_buffer(&released);
        }
        if let Some(configured_size) = client.take_configured_size_change() {
            resize_standalone_if_needed(
                &egl,
                &mut scratch,
                &mut renderer,
                &mut target,
                &mut client,
                &mut *overlay,
                &mut frames,
                config.size,
                configured_size,
                &mut size,
                &mut mapped,
                &mut last_render,
            )?;
            frames.request_fresh();
        }

        drain_screen_edge_events(
            &mut client,
            &mut *overlay,
            screen_edge.is_some(),
            &mut revealed,
            &mut frames,
        );

        deliver_platform_events(&mut *overlay, client.take_platform_events());
        deliver_localization(&mut *overlay, client.take_localization());
        deliver_settings_events(&mut client, &mut *overlay);
        deliver_alarm_events(&mut client, &mut *overlay);
        deliver_device_info_events(&mut client, &mut *overlay);

        let now = Instant::now();
        let tick =
            deliver_upgrade_update_and_tick(&mut *overlay, client.take_upgrade_update(), now);
        let want_visible = match screen_edge {
            Some(_) => screen_edge_visible(revealed, tick.visible),
            None => tick.visible,
        };
        if want_visible {
            if tick.wants_render || !mapped || client.take_needs_render() {
                frames.request_fresh();
            }
        } else {
            let _ = client.take_needs_render();
            frames.drop_request();
            if hide_standalone_if_mapped(
                &egl,
                &mut target,
                &mut client,
                &mut *overlay,
                &mut frames,
                &mut mapped,
                &mut last_render,
            )? && screen_edge.is_some()
            {
                revealed = false;
                client.rearm_screen_edge()?;
            }
        }

        let inter_frame_remaining = last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());

        render_standalone_if_ready(
            &egl,
            &mut scratch,
            &gpu_lock,
            &mut target,
            &mut renderer,
            &mut *overlay,
            &mut client,
            &mut frames,
            want_visible,
            config.size,
            &mut size,
            &mut mapped,
            &mut last_render,
            inter_frame_remaining,
            now,
        )?;

        forward_settings_requests(&mut client, &mut *overlay);
        forward_alarm_requests(&mut client, &mut *overlay);

        // A frame's cadence counts from its start, so the wait after it is measured
        // from now, not from before the render.
        let after = Instant::now();
        let floor_left = last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(after.duration_since(t)))
            .filter(|d| !d.is_zero());
        let wake = frames.wake(want_visible, tick.next_wake, after);
        client.poll_dispatch(poll_timeout_ms(
            frames.wanted(want_visible, after),
            target.available(),
            floor_left,
            wake.next_wake,
            after,
        ))?;
    }
    frames.hide(&mut *overlay);
    destroy_standalone_rendering(&egl, scratch, renderer, target);
    Ok(())
}

fn drain_screen_edge_events(
    client: &mut LayerSurfaceClient,
    overlay: &mut dyn SystemOverlay,
    has_screen_edge: bool,
    revealed: &mut bool,
    frames: &mut OverlayFrames,
) {
    if !has_screen_edge {
        return;
    }
    if client.take_reveal() {
        *revealed = true;
        overlay.on_reveal();
        frames.request_fresh();
    }
    if client.take_hidden() {
        *revealed = false;
    }
}

fn deliver_settings_events(client: &mut LayerSurfaceClient, overlay: &mut dyn SystemOverlay) {
    if !overlay.uses_settings() {
        return;
    }
    // Capabilities go first: the wire sends them first on bind, and the
    // trait promises them before any other settings event.
    if let Some(caps) = client.take_capabilities() {
        overlay.on_capabilities(caps);
    }
    if let Some(v) = client.take_brightness() {
        overlay.on_brightness(v);
    }
    if let Some(ap) = client.take_wifi_ap() {
        overlay.on_wifi_ap(ap.as_deref());
    }
    if let Some(v) = client.take_volume() {
        overlay.on_volume(v);
    }
    if let Some((active, until)) = client.take_night_mode() {
        overlay.on_night_mode(active, until.as_deref());
    }
    if let Some(reason) = client.take_restart_declined() {
        overlay.on_restart_declined(&reason);
    }
    if let Some(active) = client.take_preempted() {
        overlay.on_preempted(active);
    }
}

fn forward_settings_requests(client: &mut LayerSurfaceClient, overlay: &mut dyn SystemOverlay) {
    if !overlay.uses_settings() {
        return;
    }
    for req in overlay.drain_settings_requests() {
        if let Err(e) = client.send_settings_request(req) {
            tracing::warn!("settings request failed: {e}");
        }
    }
}

fn deliver_alarm_events(client: &mut LayerSurfaceClient, overlay: &mut dyn SystemOverlay) {
    if !overlay.uses_alarm() {
        return;
    }
    // One latest-wins slot, so a stop-then-ring within a single dispatch round
    // applies the ring (not the trailing stop) and vice versa.
    match client.take_alarm_event() {
        Some(AlarmEvent::Ring {
            time,
            period,
            label,
            snooze_allowed,
        }) => overlay.on_alarm_ring(&time, &period, &label, snooze_allowed),
        Some(AlarmEvent::Stop) => overlay.on_alarm_stop(),
        None => {}
    }
}

fn deliver_device_info_events(client: &mut LayerSurfaceClient, overlay: &mut dyn SystemOverlay) {
    if !overlay.uses_device_info() {
        return;
    }
    if let Some((state, boot_flow_delivered)) = client.take_device_state() {
        overlay.on_device_state(state, boot_flow_delivered);
    }
    if let Some((step, ssid)) = client.take_setup_progress() {
        overlay.on_setup_progress(step, &ssid);
    }
    if let Some(ap) = client.take_access_point() {
        overlay.on_access_point(ap.as_ref());
    }
    if client.take_report_ip() {
        overlay.on_report_ip();
    }
}

fn forward_alarm_requests(client: &mut LayerSurfaceClient, overlay: &mut dyn SystemOverlay) {
    if !overlay.uses_alarm() {
        return;
    }
    for req in overlay.drain_alarm_requests() {
        if let Err(e) = client.send_alarm_request(req) {
            tracing::warn!("alarm request failed: {e}");
        }
    }
}

fn poll_timeout_ms(
    frame_wanted: bool,
    target_available: bool,
    inter_frame_remaining: Option<Duration>,
    next_wake: Option<Instant>,
    now: Instant,
) -> i32 {
    let timeout = match (frame_wanted, inter_frame_remaining) {
        (true, Some(floor_left)) => Some(floor_left),
        (true, None) if target_available => Some(Duration::ZERO),
        // A wanted frame without a free buffer waits for its release,
        // which the connection reports; the tick's own wake still holds meanwhile.
        _ => next_wake.map(|t| t.saturating_duration_since(now)),
    };
    timeout.map_or(-1, |d| {
        i32::try_from(whole_millis_up(d).as_millis().max(1)).unwrap_or(i32::MAX)
    })
}

fn destroy_standalone_rendering(
    egl: &EglContext,
    scratch: SharedRenderScratch,
    renderer: FemtoVgRenderer,
    mut target: OverlayRenderTarget,
) {
    drop(renderer);
    target.destroy(egl);
    scratch.destroy(egl);
}

#[expect(
    clippy::too_many_arguments,
    reason = "resize owns the standalone GPU stack plus layer client"
)]
fn resize_standalone_if_needed(
    egl: &EglContext,
    scratch: &mut SharedRenderScratch,
    renderer: &mut FemtoVgRenderer,
    target: &mut OverlayRenderTarget,
    client: &mut LayerSurfaceClient,
    overlay: &mut dyn SystemOverlay,
    frames: &mut OverlayFrames,
    config_size: (u32, u32),
    configured_size: (u32, u32),
    size: &mut (u32, u32),
    mapped: &mut bool,
    last_render: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let new_size = resolved_configured_size(config_size, configured_size);
    if new_size == *size {
        return Ok(());
    }
    let transition = resize_transition(*mapped);
    if transition.unmap_before_resize {
        hide_standalone_if_mapped(egl, target, client, overlay, frames, mapped, last_render)?;
    }
    resize_standalone_rendering(egl, scratch, renderer, target, client, new_size)?;
    *size = new_size;
    Ok(())
}

fn hide_standalone_if_mapped(
    egl: &EglContext,
    target: &mut OverlayRenderTarget,
    client: &mut LayerSurfaceClient,
    overlay: &mut dyn SystemOverlay,
    frames: &mut OverlayFrames,
    mapped: &mut bool,
    last_render: &mut Option<Instant>,
) -> anyhow::Result<bool> {
    if !*mapped {
        return Ok(false);
    }
    client.attach_null_buffer()?;
    client.roundtrip_after_hide_unmap()?;
    target.free_for_hide(egl, client)?;
    *mapped = false;
    *last_render = None;
    frames.hide(overlay);
    Ok(true)
}

fn prepare_standalone_for_render(
    egl: &EglContext,
    scratch: &mut SharedRenderScratch,
    renderer: &mut FemtoVgRenderer,
    target: &mut OverlayRenderTarget,
    client: &mut LayerSurfaceClient,
    config_size: (u32, u32),
    size: &mut (u32, u32),
) -> anyhow::Result<()> {
    if !client.ensure_ready_for_buffer_attach()? {
        return Ok(());
    }
    let new_size = resolved_configured_size(config_size, client.size());
    if new_size != *size {
        resize_standalone_rendering(egl, scratch, renderer, target, client, new_size)?;
        *size = new_size;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "standalone render owns loop latches plus the GPU stack and layer client"
)]
fn render_standalone_if_ready(
    egl: &EglContext,
    scratch: &mut SharedRenderScratch,
    gpu_lock: &GpuRenderLock,
    target: &mut OverlayRenderTarget,
    renderer: &mut FemtoVgRenderer,
    overlay: &mut dyn SystemOverlay,
    client: &mut LayerSurfaceClient,
    frames: &mut OverlayFrames,
    visible: bool,
    config_size: (u32, u32),
    size: &mut (u32, u32),
    mapped: &mut bool,
    last_render: &mut Option<Instant>,
    inter_frame_remaining: Option<Duration>,
    now: Instant,
) -> anyhow::Result<()> {
    if !frames.wanted(visible, now) || !target.available() || inter_frame_remaining.is_some() {
        return Ok(());
    }

    prepare_standalone_for_render(egl, scratch, renderer, target, client, config_size, size)?;
    let frame = frames.frame_kind(*mapped);
    let outcome = render_frame(
        egl, scratch, gpu_lock, target, renderer, overlay, client, *size, frame,
    )?;
    frames.rendered(now, outcome);
    *mapped = true;
    *last_render = Some(now);
    Ok(())
}

fn resize_standalone_rendering(
    egl: &EglContext,
    scratch: &mut SharedRenderScratch,
    renderer: &mut FemtoVgRenderer,
    target: &mut OverlayRenderTarget,
    client: &mut LayerSurfaceClient,
    size: (u32, u32),
) -> anyhow::Result<()> {
    let new_scratch = SharedRenderScratch::new(egl, size.0, size.1)?;
    let new_renderer = create_standalone_renderer(&new_scratch, size)?;
    let old_scratch = std::mem::replace(scratch, new_scratch);
    *renderer = new_renderer;
    old_scratch.destroy(egl);
    target.resize(egl, client, size.0, size.1)?;
    Ok(())
}

fn create_standalone_renderer(
    scratch: &SharedRenderScratch,
    size: (u32, u32),
) -> anyhow::Result<FemtoVgRenderer> {
    // SAFETY: the standalone process renders on one thread with the current EGL
    // context, and `scratch` owns the FBO id used as the renderer target.
    // Its stencil lives in `EglContext::stencil_pool`, which never frees an entry,
    // so it outlives every static-layer capture.
    unsafe {
        FemtoVgRenderer::new(
            EglContext::get_proc_address,
            size.0,
            size.1,
            scratch.staging_fbo_id(),
            0,
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "render owns the full GPU stack plus connection; no meaningful grouping"
)]
fn render_frame(
    egl: &EglContext,
    scratch: &SharedRenderScratch,
    gpu_lock: &GpuRenderLock,
    target: &mut OverlayRenderTarget,
    renderer: &mut FemtoVgRenderer,
    overlay: &mut dyn SystemOverlay,
    client: &mut LayerSurfaceClient,
    size: (u32, u32),
    frame: FrameKind,
) -> anyhow::Result<RenderOutcome> {
    let lock = gpu_lock.lock("system_overlay_standalone")?;
    target.ensure_current(egl)?;
    // begin_frame binds the staging FBO and sets the viewport, returning its id.
    // The renderer was constructed against that id, so the returned value is unused.
    let _staging = scratch.begin_frame(egl, size.0, size.1);
    renderer.begin_frame_with_clear(size.0, size.1, 1.0, FrameClear::TransparentBlack);
    let outcome = overlay.render(renderer, client.viewport(size), frame);
    renderer.flush();
    scratch.blit_to(egl, target.current_fbo(), size.0, size.1);
    wait_for_gpu(egl);
    drop(lock);
    let (dmabuf, slot) = target.export_and_swap()?;
    // Mint+cache the wl_buffer for this slot, then attach. mark_presented marks
    // the slot in-flight until the compositor sends wl_buffer.release.
    let wl_buffer = target.wl_buffer_for_slot(client, &dmabuf, slot)?;
    client.submit_buffer_with_wl_buffer(&dmabuf, &wl_buffer)?;
    client.flush()?;
    target.mark_presented(slot);
    overlay.on_frame_submitted(Instant::now());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::poll_timeout_ms;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_due_frame_with_a_free_buffer_renders_on_the_next_pass() {
        let now = Instant::now();
        assert_eq!(
            poll_timeout_ms(true, true, None, Some(now + ms(2_000)), now),
            1,
            "a slow frame's successor is due at once, not a cadence later"
        );
    }

    #[test]
    fn a_due_frame_held_by_the_floor_waits_out_the_floor() {
        let now = Instant::now();
        assert_eq!(
            poll_timeout_ms(true, true, Some(ms(5)), Some(now + ms(2_000)), now),
            5
        );
    }

    #[test]
    fn a_due_frame_without_a_free_buffer_waits_for_the_tick() {
        let now = Instant::now();
        assert_eq!(
            poll_timeout_ms(true, false, None, Some(now + ms(2_000)), now),
            2_000,
            "the buffer's release arrives on the connection"
        );
    }

    #[test]
    fn an_idle_overlay_waits_for_its_tick_or_forever() {
        let now = Instant::now();
        assert_eq!(
            poll_timeout_ms(false, true, None, Some(now + ms(2_000)), now),
            2_000
        );
        assert_eq!(poll_timeout_ms(false, true, None, None, now), -1);
    }
}
