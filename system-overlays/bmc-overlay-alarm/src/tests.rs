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

use std::time::Duration;

use bmc_render::partition::test_support::move_dynamic_content;
use bmc_render::partition::tree_static_key;
use bmc_render::renderer::test_support::{DrawnArc, ShapingRecorder};
use bmc_system_overlay::{Language, Layering};

use super::*;
use bmc_system_overlay::layer_check::assert_release_frees_the_layer;

const SIZE: (u32, u32) = (480, 1280);

fn view() -> AlarmView {
    AlarmView {
        time: "7:30".to_owned(),
        period: "AM".to_owned(),
        label: "Wake up".to_owned(),
        snooze_allowed: true,
        rings: RingMotion::Ripple,
    }
}

/// An overlay built at `built_at` whose rings ripple, as the marker exists.
fn rippling_overlay(built_at: Instant) -> AlarmOverlay {
    AlarmOverlay {
        ripple_marker: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        ..AlarmOverlay::new(built_at)
    }
}

/// The first frame of `overlay` ringing.
fn first_ring_frame(mut overlay: AlarmOverlay) -> ShapingRecorder {
    overlay.on_alarm_ring("7:30", "AM", "Wake up", true);
    let mut recorder = recorder();
    let _ = overlay.render(
        &mut recorder,
        OverlayViewport::rectangular(SIZE.0, SIZE.1),
        &bmc_intl::formatter(Language::default()),
        FrameKind::Fresh,
    );
    recorder
}

#[test]
fn without_the_marker_the_rings_stand_still() {
    let overlay = AlarmOverlay {
        ripple_marker: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("no-such-marker"),
        ..AlarmOverlay::new(Instant::now())
    };
    let rings: Vec<_> = first_ring_frame(overlay)
        .arcs
        .iter()
        .map(|arc| arc.radius)
        .collect();
    assert_eq!(
        rings,
        [350.0, 425.0, 500.0, 575.0],
        "the still rings show at once"
    );
}

fn visible_arcs(recorder: &ShapingRecorder) -> Vec<&DrawnArc> {
    recorder
        .arcs
        .iter()
        .filter(|arc| !matches!(arc.fill, ArcFill::Solid(color) if color.alpha() == 0))
        .collect()
}

/// A recorder the size of the test screen.
#[expect(clippy::cast_precision_loss, reason = "the test screen fits f32")]
fn recorder() -> ShapingRecorder {
    ShapingRecorder::new(SIZE.0 as f32, SIZE.1 as f32)
}

/// Render the test alarm's `frame` into a fresh recorder and hand it back.
fn render(state: &mut AlarmRenderState, frame: OverlayFrame) -> ShapingRecorder {
    let mut recorder = recorder();
    let english = bmc_intl::formatter(Language::default());
    let _ = render_alarm(&mut recorder, SIZE, state, &view(), &english, frame);
    recorder
}

#[test]
fn the_ringing_alarm_speaks_the_language_it_was_handed() {
    const STOP: &str = "Stop Alarm";
    let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/intl/profile-languages");
    let translations = bmc_intl::Translations::load(&[profile]).0;
    let drawn_in = |language| {
        let mut overlay = AlarmOverlay::new(Instant::now());
        overlay.on_alarm_ring("7:30", "AM", "", true);
        let mut recorder = recorder();
        let _ = overlay.render(
            &mut recorder,
            OverlayViewport::rectangular(SIZE.0, SIZE.1),
            &translations.formatter(language),
            FrameKind::Fresh,
        );
        recorder
    };
    let english = drawn_in(Language::EN_GB);
    let english = english.drawn_texts();
    let chinese = drawn_in(Language::ZH);
    let chinese = chinese.drawn_texts();

    assert!(english.contains(&STOP), "{english:?}");
    assert!(
        !chinese.contains(&STOP) && chinese.len() == english.len(),
        "the alarm must speak the language it was handed: {chinese:?}"
    );
}

#[test]
fn a_ring_while_ringing_starts_the_rings_over() {
    let start = Instant::now();
    let mut overlay = AlarmOverlay::new(start);
    let at = |ms: u64| OverlayFrame {
        kind: FrameKind::Fresh,
        layering: Layering::Unlayered,
        now: start + Duration::from_millis(ms),
    };
    let mid_loop = first_loop_ms(0.5);

    overlay.on_alarm_ring("7:30", "AM", "Wake up", true);
    let _ = render(&mut overlay.render_state, at(0));
    let ringing = render(&mut overlay.render_state, at(mid_loop));
    assert!(
        !visible_arcs(&ringing).is_empty(),
        "the rings show mid-loop"
    );

    overlay.on_alarm_ring("7:35", "AM", "Wake up", true);
    let rung_again = render(&mut overlay.render_state, at(mid_loop + 10));
    let visible = visible_arcs(&rung_again);
    assert!(
        visible.is_empty(),
        "a new ring starts its rings hidden: {visible:?}"
    );
}

#[test]
fn rings_start_hidden_however_long_the_overlay_waited() {
    let an_hour_ago = Instant::now()
        .checked_sub(Duration::from_secs(3_600))
        .expect("BUG: the test runs well after boot");
    let recorder = first_ring_frame(rippling_overlay(an_hour_ago));
    let visible = visible_arcs(&recorder);
    assert!(visible.is_empty(), "the rings showed at once: {visible:?}");
}

/// The hosted overlay unmaps behind a presentation fence,
/// so `on_hidden` and the renderer release run a frame or two after the stop.
/// bmc announces a new alarm's `Started` right after the old one's `Stopped`,
/// so a ring can land in that window.
#[test]
fn a_ring_inside_the_hide_fence_strands_no_layer() {
    let english = bmc_intl::formatter(Language::default());
    let viewport = OverlayViewport::rectangular(SIZE.0, SIZE.1);
    let mut layers = recorder().with_static_layers();
    let mut overlay = rippling_overlay(Instant::now());

    overlay.on_alarm_ring("7:00", "AM", "First", true);
    let now = Instant::now();
    let _ = overlay.tick(now);
    let _ = overlay.render(&mut layers, viewport, &english, FrameKind::Fresh);

    overlay.on_alarm_stop();
    let now = now + Duration::from_millis(5);
    assert!(!overlay.tick(now).visible, "the stop hides the overlay");

    // The fence has not resolved: neither `on_hidden` nor a release has run.
    overlay.on_alarm_ring("7:05", "AM", "Second", true);
    let now = now + Duration::from_millis(20);
    assert!(overlay.tick(now).visible, "the ring cancels the hide");
    let _ = overlay.render(&mut layers, viewport, &english, FrameKind::Fresh);
    assert_eq!(
        layers.static_layer_owners().len(),
        1,
        "the restarted rings paint over the first ring's layer: {:?}",
        layers.static_layer_owners()
    );

    overlay.on_hidden();
    overlay.release_renderer_resources(&mut layers);
    assert!(
        layers.static_layer_owners().is_empty(),
        "the release frees every layer: {:?}",
        layers.static_layer_owners()
    );
}

#[test]
fn an_animation_only_frame_replays_the_alarm_tree() {
    let mut state = AlarmRenderState::new(Instant::now());
    render(&mut state, OverlayFrame::unlayered(FrameKind::Fresh));
    render(
        &mut state,
        OverlayFrame::unlayered(FrameKind::AnimationOnly),
    );
    assert!(state.tree.replayed_last_frame());
}

#[test]
fn releasing_frees_the_layer_a_ring_captured() {
    let mut overlay = AlarmOverlay::new(Instant::now());
    overlay.on_alarm_ring("7:30", "AM", "Wake up", true);
    let viewport = OverlayViewport::rectangular(SIZE.0, SIZE.1);
    assert_release_frees_the_layer(&mut overlay, SIZE, |overlay, layers| {
        let _ = overlay.render(
            layers,
            viewport,
            &bmc_intl::formatter(bmc_intl::Language::default()),
            FrameKind::Fresh,
        );
    });
}

#[test]
fn hiding_drops_the_alarm_tree() {
    let mut state = AlarmRenderState::new(Instant::now());
    render(&mut state, OverlayFrame::unlayered(FrameKind::Fresh));
    let mut overlay = AlarmOverlay::new(Instant::now());
    overlay.render_state = state;
    overlay.on_hidden();
    render(
        &mut overlay.render_state,
        OverlayFrame::unlayered(FrameKind::AnimationOnly),
    );
    assert!(!overlay.render_state.tree.replayed_last_frame());
}

fn static_key(tree: &Node) -> bmc_render::partition::StaticBytesKey {
    tree_static_key(tree).expect("BUG: the alarm tree decodes")
}

#[test]
fn a_time_change_moves_the_static_key() {
    let later = AlarmView {
        time: "7:31".to_owned(),
        ..view()
    };
    let english = bmc_intl::formatter(Language::default());
    assert_ne!(
        static_key(&build_alarm_ui_tree(&view(), SIZE, &english)),
        static_key(&build_alarm_ui_tree(&later, SIZE, &english)),
    );
}

#[test]
fn the_static_key_holds_when_only_the_rings_move() {
    let tree = build_alarm_ui_tree(&view(), SIZE, &bmc_intl::formatter(Language::default()));
    let mut moved = tree.clone();
    assert!(
        move_dynamic_content(&mut moved) > 0,
        "the rings must animate"
    );
    assert_eq!(static_key(&tree), static_key(&moved));
}

/// The alarm's frame `elapsed_ms` after it appeared.
fn frame_at(elapsed_ms: u64) -> ShapingRecorder {
    let start = Instant::now();
    let mut state = AlarmRenderState::new(start);
    let frame = OverlayFrame {
        kind: FrameKind::Fresh,
        layering: Layering::Unlayered,
        now: start + Duration::from_millis(elapsed_ms),
    };
    render(&mut state, frame)
}

/// The `(radius, opacity)` of every ring drawn `elapsed_ms` after the alarm appeared.
fn drawn_rings_at(elapsed_ms: u64) -> Vec<(f32, f32)> {
    frame_at(elapsed_ms)
        .arcs
        .iter()
        .map(|arc| {
            let ArcFill::Solid(color) = arc.fill else {
                panic!("BUG: the rings are solid");
            };
            (arc.radius, f32::from(color.alpha()) / 255.0)
        })
        .collect()
}

fn assert_near(actual: f32, expected: f32, tolerance: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} is not within {tolerance} of {expected}"
    );
}

/// The radius of a ring at `pct` percent of the test screen's longer side.
#[expect(clippy::cast_precision_loss, reason = "the test screen fits f32")]
fn radius_at(pct: f32) -> f32 {
    SIZE.0.max(SIZE.1) as f32 * pct / 100.0
}

/// Milliseconds into a ring's first loop after `share` of the loop.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a share of a loop is a short positive span"
)]
fn first_loop_ms(share: f32) -> u64 {
    u64::from(RING_HIDDEN_MS) + (share * f32::from(RING_LOOP_MS.get())).round() as u64
}

/// The `(radius, opacity)` of the first ring, which is drawn first.
fn first_ring_at(elapsed_ms: u64) -> (f32, f32) {
    drawn_rings_at(elapsed_ms)[0]
}

#[test]
fn a_ring_appears_between_the_centre_and_its_opaque_radius() {
    let appear = appear_at_pct();
    assert!(
        appear > 0.0 && appear < f32::from(RING_OPAQUE_AT_PCT),
        "a ring appearing at {appear} % would scale by a negative or infinite factor: \
         shorten RING_FADE_IN_MS or bring RING_OPAQUE_AT_PCT closer to RING_VANISH_AT_PCT"
    );
}

#[test]
fn no_ring_shows_before_the_first_appears() {
    let frame = frame_at(u64::from(RING_HIDDEN_MS) - 1);
    let visible = visible_arcs(&frame);
    assert!(visible.is_empty(), "a ring showed early: {visible:?}");
}

#[test]
fn the_first_ring_turns_opaque_at_its_opaque_radius() {
    let (radius, opacity) = first_ring_at(first_loop_ms(RING_FADE_IN_SHARE));
    assert_near(radius, radius_at(RING_OPAQUE_AT_PCT.into()), 0.5, "radius");
    assert_near(opacity, 1.0, 0.02, "opacity");
}

#[test]
fn a_ring_fades_out_in_step_with_its_radius() {
    let (radius, opacity) = first_ring_at(first_loop_ms(0.5));
    let opaque = radius_at(RING_OPAQUE_AT_PCT.into());
    let vanish = radius_at(RING_VANISH_AT_PCT.into());
    let left = (vanish - radius) / (vanish - opaque);
    assert_near(opacity, left, 0.01, "opacity");
}

#[test]
fn a_ring_vanishes_at_its_vanish_radius() {
    let (radius, opacity) = first_ring_at(first_loop_ms(1.0) - 10);
    assert_near(radius, radius_at(RING_VANISH_AT_PCT.into()), 1.0, "radius");
    assert_near(opacity, 0.0, 0.01, "opacity");
}

#[test]
fn every_later_loop_fades_in_again() {
    let second_loop = first_loop_ms(1.0 + RING_FADE_IN_SHARE / 2.0);
    let (_, opacity) = first_ring_at(second_loop);
    assert!(
        opacity > 0.0 && opacity < 1.0,
        "a ring halfway through its second fade-in is partly faded in, not {opacity}"
    );
}

#[test]
fn every_ring_shows_once_the_ripple_is_running() {
    assert_eq!(
        visible_arcs(&frame_at(first_loop_ms(3.1))).len(),
        usize::from(RING_COUNT.get())
    );
}

#[test]
fn every_alarm_on_every_display_has_a_static_key() {
    let views = [
        view(),
        AlarmView {
            snooze_allowed: false,
            ..view()
        },
        AlarmView {
            label: String::new(),
            ..view()
        },
    ];
    let english = bmc_intl::formatter(Language::default());
    for size in [SIZE, (1_280, 480), (480, 480), (480, 320), (320, 240)] {
        for view in &views {
            assert!(
                tree_static_key(&build_alarm_ui_tree(view, size, &english)).is_ok(),
                "{view:?} at {size:?}"
            );
        }
    }
}
