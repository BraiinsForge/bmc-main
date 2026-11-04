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

use super::{Layering, TreeUi};
use crate::overlay::{FrameKind, NextFrame, RenderOutcome, TouchEvent};
use crate::test_support::TestRenderer;
use bmc_render::partition::test_support::add_animated_draw;
use bmc_render::renderer::test_support::ShapingRecorder;
use bmc_render::repaint::TargetContents;
use bmc_render::test_tracing::counting_warns;
use bmc_render::tree::Node;
use bmc_render::tree::StaticLayer;
use bmc_render::ui::{Color, PropsData};

fn english() -> bmc_intl::Intl {
    bmc_intl::formatter(bmc_intl::Language::default())
}

/// A full-width ProgressBar carrying the brightness touch key.
fn slider_tree() -> Node {
    Node::ProgressBar {
        touch_key: Some("brightness".to_owned()),
        track_h: 8.0,
        mode: bmc_render::tree::ProgressMode::Slider(0.0),
        active: true,
        fill_color: Color::from_rgba(0, 255, 0, 255),
        track_color: Color::from_rgba(80, 80, 80, 255),
        bg_color: Color::from_rgba(0, 0, 0, 0),
        skin: None,
    }
}

/// An indeterminate bar, which the renderer animates on its own.
fn animated_tree() -> Node {
    Node::ProgressBar {
        touch_key: None,
        track_h: 8.0,
        mode: bmc_render::tree::ProgressMode::Indeterminate,
        active: true,
        fill_color: Color::from_rgba(0, 255, 0, 255),
        track_color: Color::from_rgba(80, 80, 80, 255),
        bg_color: Color::from_rgba(0, 0, 0, 0),
        skin: None,
    }
}

/// Render one frame, counting how often the tree was built.
fn frame(
    ui: &mut TreeUi,
    kind: FrameKind,
    size: (u32, u32),
    builds: &mut usize,
) -> super::TreeFrame {
    ui.render_after(
        kind,
        Layering::Unlayered,
        size,
        16,
        &mut TestRenderer::default(),
        &english(),
        || {
            *builds += 1;
            animated_tree()
        },
    )
    .expect("BUG: render must succeed")
}

#[test]
fn touch_on_slider_reports_drag_fraction() {
    let mut ui = TreeUi::default();
    let mut r = TestRenderer::default();
    // Warm-up frame establishes the slider's hit region (regions are
    // prior-frame state); the touch is hit-tested on the next frame.
    ui.render_after(
        FrameKind::Fresh,
        Layering::Unlayered,
        (100, 20),
        16,
        &mut r,
        &english(),
        slider_tree,
    )
    .expect("BUG: warm-up render must succeed");
    // A 100px-wide frame; touch down at x≈75 → fraction ≈ 0.75.
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 75.0,
        y: 10.0,
    });
    let result = ui
        .render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut r,
            &english(),
            slider_tree,
        )
        .expect("BUG: hit-test render must succeed");
    let hit = result
        .result
        .drags
        .get("brightness")
        .expect("slider drag should be hit-tested");
    let frac = (hit.x / hit.width).clamp(0.0, 1.0);
    assert!(
        (0.6..=0.9).contains(&frac),
        "expected ~0.75, got {frac} (x={}, w={})",
        hit.x,
        hit.width
    );
}

#[test]
fn touch_off_the_slider_reports_no_drag() {
    let mut ui = TreeUi::default();
    let mut r = TestRenderer::default();
    // No touch pushed → no drag entry.
    let result = ui
        .render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut r,
            &english(),
            slider_tree,
        )
        .expect("BUG: render must succeed");
    assert!(!result.result.drags.contains_key("brightness"));
}

#[test]
fn coalesced_down_up_on_slider_reports_release_click() {
    let mut ui = TreeUi::default();
    let mut r = TestRenderer::default();
    // Warm-up frame establishes the slider's hit region.
    ui.render_after(
        FrameKind::Fresh,
        Layering::Unlayered,
        (100, 20),
        16,
        &mut r,
        &english(),
        slider_tree,
    )
    .expect("BUG: warm-up render must succeed");
    // Down + Up drained into one frame (no render between) — the host's
    // event-coalescing case. The release leaves no `drags` entry, but the
    // click still hit-tests the finger-up position so the final value is
    // recoverable.
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 75.0,
        y: 10.0,
    });
    ui.push_touch(TouchEvent::Up { id: 0 });
    let result = ui
        .render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut r,
            &english(),
            slider_tree,
        )
        .expect("BUG: hit-test render must succeed");
    assert!(
        !result.result.drags.contains_key("brightness"),
        "a released touch must not report an active drag"
    );
    let hit = result
        .result
        .clicks
        .get("brightness")
        .expect("a coalesced down-up on the slider must report a release click");
    let frac = (hit.x / hit.width).clamp(0.0, 1.0);
    assert!(
        (0.6..=0.9).contains(&frac),
        "expected ~0.75, got {frac} (x={}, w={})",
        hit.x,
        hit.width
    );
}

#[test]
fn an_animation_only_frame_replays_the_cached_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
    assert_eq!(builds, 1, "the replayed frame must not build a tree");
    assert!(ui.replayed_last_frame());
}

#[test]
fn a_fresh_frame_builds_a_new_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    assert_eq!(builds, 2);
    assert!(!ui.replayed_last_frame());
}

#[test]
fn an_animation_only_frame_builds_without_a_cached_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
    assert_eq!(builds, 1);
}

#[test]
fn a_size_change_builds_a_new_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    frame(&mut ui, FrameKind::AnimationOnly, (200, 20), &mut builds);
    assert_eq!(builds, 2, "the cached tree was laid out for another size");
}

#[test]
fn a_pending_touch_builds_a_new_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 50.0,
        y: 10.0,
    });
    frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
    assert_eq!(builds, 2);
}

#[test]
fn a_restart_keeps_a_pending_touch() {
    let mut ui = TreeUi::default();
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 75.0,
        y: 10.0,
    });
    ui.restart();
    assert!(
        ui.walk.interaction.has_pending_events(),
        "a tap under way must survive a restart"
    );
}

#[test]
fn clear_drops_the_cached_tree() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    ui.clear();
    frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
    assert_eq!(builds, 2);
}

#[test]
fn a_running_animation_is_reported_on_replay() {
    let mut ui = TreeUi::default();
    let mut builds = 0;
    frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
    let replayed = frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
    assert_eq!(replayed.outcome().next, NextFrame::Replay);
}

/// A canvas with one arc rotating forever.
fn rotating() -> Node {
    use bmc_render::tree::{AnimationDef, Draw};
    use bmc_render::ui::{AnimProperty, Easing, LoopMode};
    Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![Draw::Modified {
            animations: vec![AnimationDef {
                property: AnimProperty::Rotate,
                from: 0.0,
                to: 1.0,
                duration_ms: 1_000,
                delay_ms: 0,
                easing: Easing::Linear,
                loop_mode: LoopMode::Forever,
            }],
            transition: None,
            color_space: bmc_render::ui::ColorSpace::default(),
            inner: Box::new(Draw::Arc {
                cx: 20.0,
                cy: 10.0,
                radius: 8.0,
                start_angle: 0.0,
                end_angle: 1.0,
                width: 2.0,
                fill: bmc_render::ui::ArcFill::Solid(GREY),
                segments: bmc_render::ui::ArcSegments::Continuous,
                cap: bmc_render::ui::ArcCap::Round,
            }),
        }],
    }
}

/// Render [`rotating`] as a fresh frame at `now`.
fn render_rotating_at(ui: &mut TreeUi, now: std::time::Instant) {
    let frame = super::OverlayFrame {
        kind: FrameKind::Fresh,
        layering: Layering::Unlayered,
        now,
    };
    ui.render(
        frame,
        (100, 20),
        &mut TestRenderer::default(),
        &english(),
        rotating,
    )
    .expect("BUG: render must succeed");
}

fn animation_elapsed_ms(ui: &TreeUi) -> Vec<u32> {
    ui.walk
        .animation_states
        .values()
        .map(|s| s.elapsed_ms)
        .collect()
}

#[test]
fn a_frame_advances_animations_by_the_time_since_the_last() {
    use std::time::{Duration, Instant};
    let t0 = Instant::now();
    let mut ui = TreeUi::new(t0);
    for ms in [40, 100] {
        render_rotating_at(&mut ui, t0 + Duration::from_millis(ms));
    }
    assert_eq!(animation_elapsed_ms(&ui), [100]);
}

#[test]
fn a_restart_starts_every_animation_over() {
    use std::time::{Duration, Instant};
    let t0 = Instant::now();
    let mut ui = TreeUi::new(t0);
    render_rotating_at(&mut ui, t0 + Duration::from_millis(400));
    ui.restart();
    render_rotating_at(&mut ui, t0 + Duration::from_millis(1_000));
    assert_eq!(
        animation_elapsed_ms(&ui),
        [0],
        "the first frame after a restart starts the animation from its beginning"
    );
}

#[test]
fn a_still_tree_reports_no_animation() {
    let mut ui = TreeUi::default();
    let frame = ui
        .render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut TestRenderer::default(),
            &english(),
            slider_tree,
        )
        .expect("BUG: render must succeed");
    assert_eq!(frame.outcome().next, NextFrame::Idle);
}

const SIZE: (u32, u32) = (100, 20);
const GREY: Color = Color::from_rgb(40, 40, 40);
const BLUE: Color = Color::from_rgb(0, 0, 200);

/// A static panel fill around `bar`, which moves or takes touches.
fn panel(background: Color, bar: Node) -> Node {
    Node::Column(
        PropsData {
            background,
            ..PropsData::default()
        },
        vec![bar],
    )
}

fn layers() -> ShapingRecorder {
    ShapingRecorder::new(100.0, 20.0).with_static_layers()
}

fn layered(
    ui: &mut TreeUi,
    renderer: &mut ShapingRecorder,
    kind: FrameKind,
    size: (u32, u32),
    tree: &Node,
) -> StaticLayer {
    ui.render_after(
        kind,
        Layering::Layered,
        size,
        16,
        renderer,
        &english(),
        || tree.clone(),
    )
    .expect("BUG: render must succeed")
    .profile
    .static_layer
}

#[cfg(feature = "frame-timings")]
#[test]
fn only_the_frame_that_keys_the_tree_reports_its_cost() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    // Enough labels that encoding and decoding the tree takes well over a microsecond.
    let labels = (0..200)
        .map(|i| bmc_render::ui::text(format!("label {i}"), bmc_render::ui::TextStyle::default()));
    let tree = Node::Column(
        PropsData {
            background: GREY,
            ..PropsData::default()
        },
        labels.collect(),
    );
    let mut key_us = |kind| {
        ui.render_after(
            kind,
            Layering::Layered,
            SIZE,
            16,
            &mut r,
            &english(),
            || tree.clone(),
        )
        .expect("BUG: render must succeed")
        .profile
        .timings
        .deserialize_us
    };
    assert!(
        key_us(FrameKind::Fresh) > 0,
        "the fresh frame keys the tree"
    );
    assert_eq!(
        key_us(FrameKind::AnimationOnly),
        0,
        "a replay reuses the key the tree was given"
    );
}

#[test]
fn a_layered_frame_captures_the_static_layer() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
    assert_eq!(r.static_layer_owners(), [ui.static_layer_owner().clone()]);
}

#[test]
fn an_unlayered_frame_paints_the_tree_without_a_layer() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let outcome = ui
        .render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            SIZE,
            16,
            &mut r,
            &english(),
            || panel(GREY, animated_tree()),
        )
        .expect("BUG: render must succeed")
        .profile
        .static_layer;
    assert_eq!(outcome, StaticLayer::Unused);
    assert!(r.static_layer_owners().is_empty());
}

#[test]
fn a_fresh_tree_with_the_same_static_half_reuses_the_layer() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    assert_eq!(outcome, StaticLayer::Blitted);
}

#[test]
fn a_replayed_tree_reuses_the_layer() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
    assert_eq!(outcome, StaticLayer::Blitted);
}

#[test]
fn a_static_edit_captures_again() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    layered(
        &mut ui,
        &mut r,
        FrameKind::Fresh,
        SIZE,
        &panel(GREY, animated_tree()),
    );
    let edited = panel(BLUE, animated_tree());
    let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &edited);
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
}

/// The panel around a live relative time anchored `ago_secs` before now,
/// whose label sizes the static fill around it.
fn panel_with_age(ago_secs: i64) -> Node {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("BUG: the clock is past the epoch")
        .as_secs()
        .cast_signed();
    let age = bmc_render::ui::relative_time_live(
        bmc_render::ui::SystemTime {
            unix_secs: now - ago_secs,
        },
        bmc_render::ui::RelTimeFormat::try_from(0).expect("BUG: 0 is a valid format"),
        bmc_render::ui::RelTimeClamp::default(),
        bmc_render::ui::TextStyle::default(),
    );
    Node::Row(
        PropsData {
            background: GREY,
            ..PropsData::default()
        },
        vec![age],
    )
}

#[test]
fn a_layout_change_alone_captures_again() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let recent = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &panel_with_age(5));
    let old = layered(
        &mut ui,
        &mut r,
        FrameKind::Fresh,
        SIZE,
        &panel_with_age(3_600),
    );
    assert_eq!(
        [recent, old],
        [StaticLayer::Captured { empty: false }; 2],
        "a wider label moves the fill it sits in, though the static bytes did not change"
    );
}

#[test]
fn a_size_change_captures_again() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, (200, 20), &tree);
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
}

#[test]
fn a_press_captures_again_and_holds_while_it_lasts() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, slider_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 50.0,
        y: 4.0,
    });
    let pressed = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    assert!(
        ui.is_pressed("brightness"),
        "the touch must land on the slider"
    );
    assert_eq!(pressed, StaticLayer::Captured { empty: false });
    let held = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
    assert_eq!(held, StaticLayer::Blitted);
}

#[test]
fn a_cancelled_press_captures_again_on_replay() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, slider_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 50.0,
        y: 4.0,
    });
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.cancel_touch();
    let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
    assert!(ui.replayed_last_frame());
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
}

#[test]
fn a_drag_over_a_scroll_container_captures_again() {
    let list = bmc_render::ui::scroll("list", PropsData::default(), [slider_tree()]);
    let tree = panel(GREY, list);
    let mut ui = TreeUi::default();
    let mut r = layers();
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.push_touch(TouchEvent::Down {
        id: 0,
        x: 50.0,
        y: 4.0,
    });
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.push_touch(TouchEvent::Motion {
        id: 0,
        x: 50.0,
        y: 6.0,
    });
    assert_eq!(
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree),
        StaticLayer::Captured { empty: false }
    );
}

#[test]
fn an_unkeyed_tree_paints_without_a_layer_and_frees_it() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    layered(
        &mut ui,
        &mut r,
        FrameKind::Fresh,
        SIZE,
        &panel(GREY, animated_tree()),
    );
    let negative_extent = panel(GREY, bmc_render::ui::spacer(-1.0));
    let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &negative_extent);
    assert_eq!(outcome, StaticLayer::Unused);
    assert!(r.static_layer_owners().is_empty());
}

#[test]
fn an_unkeyed_run_warns_once_until_a_tree_keys_again() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let unkeyed = panel(GREY, bmc_render::ui::spacer(-1.0));
    let keyed = panel(GREY, animated_tree());
    let ((), warnings) = counting_warns(|_| {
        for _ in 0..3 {
            layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &unkeyed);
        }
    });
    assert_eq!(warnings, 1, "a run of unkeyed trees warns once");
    let ((), warnings) = counting_warns(|_| {
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &keyed);
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &unkeyed);
    });
    assert_eq!(warnings, 1, "a keyed tree re-arms the warning");
}

#[test]
fn a_tree_built_unlayered_keys_on_its_first_layered_replay() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    ui.render_after(
        FrameKind::Fresh,
        Layering::Unlayered,
        SIZE,
        16,
        &mut r,
        &english(),
        || tree.clone(),
    )
    .expect("BUG: render must succeed");
    let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
    assert!(ui.replayed_last_frame());
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
}

#[test]
fn an_unlayered_frame_never_keys_its_tree() {
    let mut ui = TreeUi::default();
    let unkeyed = panel(GREY, bmc_render::ui::spacer(-1.0));
    let ((), warnings) = counting_warns(|_| {
        ui.render_after(
            FrameKind::Fresh,
            Layering::Unlayered,
            SIZE,
            16,
            &mut layers(),
            &english(),
            || unkeyed.clone(),
        )
        .expect("BUG: render must succeed");
    });
    assert_eq!(warnings, 0, "a tree never keyed cannot fail to key");
}

#[test]
fn a_miss_stops_capturing_until_release() {
    let mut ui = TreeUi::default();
    let mut refusing = ShapingRecorder::new(100.0, 20.0);
    let tree = panel(GREY, animated_tree());
    let missed = layered(&mut ui, &mut refusing, FrameKind::Fresh, SIZE, &tree);
    assert_eq!(missed, StaticLayer::Missed);
    let stopped = layered(
        &mut ui,
        &mut refusing,
        FrameKind::AnimationOnly,
        SIZE,
        &tree,
    );
    assert!(ui.replayed_last_frame(), "a miss keeps the tree");
    assert_eq!(stopped, StaticLayer::Unused);
    ui.release(&mut refusing);
    let retried = layered(&mut ui, &mut refusing, FrameKind::Fresh, SIZE, &tree);
    assert_eq!(retried, StaticLayer::Missed);
}

#[test]
fn a_failed_walk_stops_counting_on_the_layer() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.fail_next_walk = true;
    assert!(
        ui.render_after(
            FrameKind::Fresh,
            Layering::Layered,
            SIZE,
            16,
            &mut r,
            &english(),
            || { tree.clone() }
        )
        .is_err()
    );
    let after = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    assert_eq!(
        after,
        StaticLayer::Captured { empty: false },
        "a failed walk forgets the layer, since a real one may stop halfway through a capture"
    );
}

#[test]
fn release_frees_the_layer_and_the_next_frame_captures_it() {
    let mut ui = TreeUi::default();
    let mut r = layers();
    let tree = panel(GREY, animated_tree());
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
    ui.release(&mut r);
    assert!(r.static_layer_owners().is_empty());
    let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
    assert_eq!(outcome, StaticLayer::Captured { empty: false });
}

#[test]
fn two_trees_capture_under_their_own_keys() {
    let (mut first, mut second) = (TreeUi::default(), TreeUi::default());
    let mut r = layers();
    layered(
        &mut first,
        &mut r,
        FrameKind::Fresh,
        SIZE,
        &panel(GREY, animated_tree()),
    );
    layered(
        &mut second,
        &mut r,
        FrameKind::Fresh,
        SIZE,
        &panel(BLUE, animated_tree()),
    );
    assert_ne!(first.static_layer_owner(), second.static_layer_owner());
    assert_eq!(r.static_layer_owners().len(), 2);
    let outcome = layered(
        &mut first,
        &mut r,
        FrameKind::AnimationOnly,
        SIZE,
        &panel(GREY, animated_tree()),
    );
    assert_eq!(outcome, StaticLayer::Blitted);
}

/// A static panel with a spinner on a small canvas, which damages well under the cap,
/// as the indeterminate bar spanning the surface does not.
/// The spinner's rotation is unbounded, so its damage is the whole canvas.
fn spinner_panel() -> Node {
    let mut canvas = Node::Canvas {
        props: PropsData {
            width: 20.0,
            height: 10.0,
            ..PropsData::default()
        },
        touch_key: None,
        draws: Vec::new(),
    };
    assert!(add_animated_draw(&mut canvas));
    panel(GREY, canvas)
}

/// A tree past its capture, with a replay that caught the other buffer up on it,
/// so its next replay may keep the target.
fn settled(r: &mut ShapingRecorder) -> (TreeUi, Node) {
    let mut ui = TreeUi::default();
    let tree = spinner_panel();
    layered(&mut ui, r, FrameKind::Fresh, SIZE, &tree);
    layered(&mut ui, r, FrameKind::AnimationOnly, SIZE, &tree);
    (ui, tree)
}

#[test]
fn a_settled_replay_keeps_the_target_and_repaints_only_what_moves() {
    let mut r = layers();
    let (mut ui, tree) = settled(&mut r);
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Preserved
    );
    let repainted = ui
        .render_after(
            FrameKind::AnimationOnly,
            Layering::Layered,
            SIZE,
            16,
            &mut r,
            &english(),
            || tree.clone(),
        )
        .expect("BUG: render must succeed")
        .profile
        .repainted_px;
    let surface = usize::try_from(SIZE.0 * SIZE.1).expect("BUG: a test surface fits usize");
    assert!(
        repainted < surface,
        "the replay scissors to the spinner, got {repainted} px"
    );
}

#[test]
fn the_capture_owes_the_other_buffer_a_full_repaint() {
    let mut r = layers();
    let mut ui = TreeUi::default();
    layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &spinner_panel());
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Cleared
    );
}

#[test]
fn a_fresh_frame_never_keeps_the_target() {
    let mut r = layers();
    let (mut ui, _) = settled(&mut r);
    assert_eq!(
        ui.prepare_target(FrameKind::Fresh, SIZE),
        TargetContents::Cleared
    );
}

#[test]
fn every_way_a_buffer_is_lost_repaints_both_in_full() {
    type Loss = fn(&mut TreeUi, &mut ShapingRecorder, &Node);
    let cases: [(&str, Loss, FrameKind); 3] = [
        (
            "invalidated",
            |ui, _, _| ui.invalidate_target(),
            FrameKind::AnimationOnly,
        ),
        ("hidden", |ui, _, _| ui.clear(), FrameKind::Fresh),
        (
            "failed",
            |ui, r, tree| {
                ui.fail_next_walk = true;
                let failed = ui.render_after(
                    FrameKind::Fresh,
                    Layering::Layered,
                    SIZE,
                    16,
                    r,
                    &english(),
                    || tree.clone(),
                );
                assert!(failed.is_err());
            },
            FrameKind::Fresh,
        ),
    ];
    for (loss, lose, first) in cases {
        let mut r = layers();
        let (mut ui, tree) = settled(&mut r);
        lose(&mut ui, &mut r, &tree);
        assert_eq!(
            ui.layered.damage().owed_full_repaints(),
            bmc_render::repaint::EXPORT_BUFFERS,
            "{loss}: the first frame's capture would hide a debt owed to only one buffer"
        );
        for kind in [first, FrameKind::AnimationOnly] {
            assert_eq!(
                ui.prepare_target(kind, SIZE),
                TargetContents::Cleared,
                "{loss}: a {kind:?} frame may find its buffer holding anything"
            );
            layered(&mut ui, &mut r, kind, SIZE, &tree);
        }
        assert_eq!(
            ui.prepare_target(FrameKind::AnimationOnly, SIZE),
            TargetContents::Preserved,
            "{loss}: both buffers hold a full frame again"
        );
    }
}

/// The host kept the target for a replay, so a frame that builds instead
/// would draw the new tree over the old one.
#[test]
fn a_kept_target_refuses_a_frame_that_is_not_its_replay() {
    let mut r = layers();
    let (mut ui, tree) = settled(&mut r);
    ui.prepare_target(FrameKind::AnimationOnly, SIZE);
    let refused = ui.render_after(
        FrameKind::Fresh,
        Layering::Layered,
        SIZE,
        16,
        &mut r,
        &english(),
        || tree.clone(),
    );
    assert!(refused.is_err());
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Cleared,
        "the refused frame left the buffer as it was, which no longer matches the history"
    );
}

/// Paint a `kind` frame of `ui` the way a host does, drawing no tree,
/// and say what its target was found to hold.
fn paint_no_tree(ui: &mut TreeUi, r: &mut ShapingRecorder, kind: FrameKind) -> TargetContents {
    let mut found = None;
    let _ = crate::paint_surface_frame(
        ui,
        |ui| Some(ui),
        r,
        SIZE,
        kind,
        |contents| {
            found = Some(contents);
            Ok(())
        },
        |_, _| RenderOutcome::default(),
    )
    .expect("BUG: a recorded frame always binds");
    found.expect("BUG: paint_surface_frame binds every frame")
}

/// The kept buffer still shows the frame before last, so the host clears it after all,
/// and neither buffer holds what the damage history describes.
#[test]
fn a_kept_frame_that_draws_no_tree_is_cleared_and_repaints_both_buffers() {
    let mut r = layers();
    let (mut ui, _) = settled(&mut r);
    assert_eq!(
        paint_no_tree(&mut ui, &mut r, FrameKind::AnimationOnly),
        TargetContents::Preserved,
        "the fixture must be a frame the host keeps the buffer for"
    );
    assert_eq!(r.clears, 1);
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Cleared
    );
}

/// The buffer a frame failed to bind may hold anything by the time a later frame gets it.
#[test]
fn a_frame_that_fails_to_bind_repaints_both_buffers() {
    let mut r = layers();
    let (mut ui, _) = settled(&mut r);
    let painted = crate::paint_surface_frame(
        &mut ui,
        |ui| Some(ui),
        &mut r,
        SIZE,
        FrameKind::AnimationOnly,
        |contents| {
            assert_eq!(
                contents,
                TargetContents::Preserved,
                "the fixture must be a frame the host keeps the buffer for"
            );
            anyhow::bail!("the export buffer could not be allocated")
        },
        |_, _| panic!("a frame that failed to bind draws nothing"),
    );
    assert!(painted.is_err());
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Cleared
    );
}

/// A cleared buffer the overlay drew no tree into is exported all the same,
/// so it no longer holds the frame the damage history says it does.
#[test]
fn a_cleared_frame_that_draws_no_tree_repaints_both_buffers() {
    let mut r = layers();
    let (mut ui, _) = settled(&mut r);
    assert_eq!(
        paint_no_tree(&mut ui, &mut r, FrameKind::Fresh),
        TargetContents::Cleared
    );
    assert_eq!(r.clears, 0, "a cleared frame needs no second clear");
    assert_eq!(
        ui.prepare_target(FrameKind::AnimationOnly, SIZE),
        TargetContents::Cleared,
        "the replay that follows must not trust its buffer"
    );
}
