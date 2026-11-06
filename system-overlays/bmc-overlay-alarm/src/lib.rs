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

//! Full-screen firing-alarm overlay.
//!
//! Maps a full-screen surface while an alarm is ringing (driven by the
//! `deck_alarm_v1` `alarm_ringing` event) and unmaps once the user snoozes or
//! dismisses, or the compositor reports the alarm stopped. Snooze/dismiss taps
//! are sent back over the same protocol.

use std::num::NonZeroU16;
use std::path::PathBuf;
use std::time::Instant;
use std::vec;

use bmc_intl::{Intl, format_message};
use bmc_render::animation::apply_easing;
use bmc_render::renderer::Renderer;
use bmc_render::ui::{
    AnimProperty, ArcCap, ArcFill, ArcSegments, BLACK, Color, CrossAlign, Draw, Easing, Fill,
    FontFamily, FontWeight, GRAY_30, Interpolation, LoopMode, Node, ORANGE_30, ORANGE_40,
    ORANGE_50, PathPaint, PropsData, TRANSPARENT, TextAlign, TextStyle, VerticalAlign, WHITE, col,
    fixed_height, row, text,
};
use bmc_system_overlay::{
    AlarmRequest, Anchor, FrameKind, InputRegion, Layer, LayerConfig, OverlayFrame,
    OverlayViewport, RenderOutcome, SystemOverlay, TickOutcome, TouchEvent, TreeUi,
};

/// Touch key for the stop-alarm button; shared by its `Canvas` hit region and
/// the `TreeResult::clicks` lookup so `render` and the click handler agree.
const STOP_ALARM_KEY: &str = "stop_alarm_button";
const SNOOZE_ALARM_KEY: &str = "snooze_alarm_button";

/// The rings ripple only while this file exists, until the animation is approved.
const RIPPLE_MARKER: &str = "/etc/bmc/alarm-animation";

/// The firing alarm currently shown, as delivered by the `alarm_ringing` event.
#[derive(Debug)]
struct Ring {
    /// Scheduled alarm time, preformatted for display (e.g. `07:30`).
    time: String,
    /// AM/PM marker for 12-hour time; empty in 24-hour mode.
    period: String,
    /// Alarm label; empty when the alarm has no name, in which case the screen
    /// falls back to "Alarm" (see `build_alarm_ui_tree`).
    label: String,
    /// Whether the alarm may be snoozed; `false` hides the snooze button.
    snooze_allowed: bool,
    motion: RingMotion,
}

/// Keeps context for UI rendering.
#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct AlarmRenderState {
    tree: TreeUi,
}

impl AlarmRenderState {
    #[must_use]
    pub fn new(now: Instant) -> AlarmRenderState {
        Self {
            tree: TreeUi::new(now),
        }
    }
}

/// Full-screen overlay shown while an alarm is ringing.
#[expect(
    missing_debug_implementations,
    reason = "AlarmRenderState is not Debug"
)]
pub struct AlarmOverlay {
    /// `Some` while an alarm is ringing; `None` means nothing to show. This is
    /// the single source of truth for visibility (see `tick`).
    ringing: Option<Ring>,
    /// Requests to hand back to the compositor next pass. The framework drains
    /// this via `drain_alarm_requests` after `render`.
    pending: Vec<AlarmRequest>,
    /// Content changed since the last paint; asks the framework for a redraw.
    dirty: bool,
    /// UI rendering context.
    render_state: AlarmRenderState,
    /// Read at each ring, so creating or removing it takes effect from the next alarm.
    ripple_marker: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AlarmView {
    pub time: String,
    /// AM/PM marker rendered smaller next to the time; empty in 24-hour mode.
    pub period: String,
    /// Alarm label; empty falls back to "Alarm" at render (see `build_alarm_ui_tree`).
    pub label: String,
    /// Whether the snooze button is shown; `false` renders stop-only.
    pub snooze_allowed: bool,
    /// Whether the rings behind the alarm ripple or stand still.
    pub rings: RingMotion,
}

/// How the rings behind the alarm move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingMotion {
    /// The rings ripple out from the centre.
    Ripple,
    /// The rings stand still.
    Still,
}

/// A 400×100 alarm action button: a solid `fill` background, an optional 1px
/// `border`, and a centred white 32px Deck Sans SemiBold `label`. `key` is the
/// `Canvas` touch key, echoed back in `TreeResult::clicks` on a tap.
///
/// Hand-rolled from a `Canvas` instead of `make_button` because the stock
/// `Button` node is not customizable enough for this screen. Its appearance is
/// fixed by `ButtonStyle` — a five-colour palette (Primary/Secondary/Danger/
/// Tertiary/Ghost) with no orange and no per-instance background or border
/// colour; the only escape is a 9-patch bitmap `skin`, which is overkill for a
/// flat fill. A `Canvas` instead carries arbitrary draw commands (fill, border,
/// label) *and* a `touch_key`, so we get the custom look while the tap still
/// flows through the framework's normal hit-testing.
fn alarm_button(key: &str, label: &str, fill: Fill, border: Option<Color>) -> Node {
    /// Logical size of every alarm action button.
    const BUTTON_W: f32 = 400.0;
    const BUTTON_H: f32 = 100.0;

    let mut draws = vec![Draw::Rect {
        x: 0.0,
        y: 0.0,
        w: BUTTON_W,
        h: BUTTON_H,
        fill,
    }];

    // Optional 1px border. Inset half the stroke width so the centred stroke
    // stays fully inside the canvas rather than clipping at the edge; `closed`
    // joins the last corner back to the first.
    if let Some(color) = border {
        draws.push(Draw::Path {
            points: vec![
                (0.5, 0.5),
                (BUTTON_W - 0.5, 0.5),
                (BUTTON_W - 0.5, BUTTON_H - 0.5),
                (0.5, BUTTON_H - 0.5),
            ],
            paint: PathPaint::Stroke {
                color,
                width: 1.0,
                dash: None,
            },
            closed: true,
            interpolation: Interpolation::Linear,
        });
    }

    // Placed at the canvas centre; `align`/`vertical_align: Center` anchor the
    // glyph box on that point rather than its top-left. Pushed last so the
    // label paints on top of the fill and border.
    draws.push(Draw::Text {
        x: BUTTON_W / 2.0,
        y: BUTTON_H / 2.0,
        text: label.to_owned(),
        style: TextStyle {
            size: 32,
            color: WHITE,
            weight: FontWeight::SEMIBOLD,
            align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            family: FontFamily::DeckSans,
            ..Default::default()
        },
    });

    Node::Canvas {
        props: PropsData {
            width: BUTTON_W,
            height: BUTTON_H,
            ..Default::default()
        },
        touch_key: Some(key.to_owned()),
        draws,
    }
}

fn draw_outline_circle(cx: f32, cy: f32, r: f32, color: Color) -> Draw {
    Draw::Arc {
        cx,
        cy,
        radius: r,
        start_angle: 0.0,
        end_angle: std::f32::consts::TAU,
        width: 1.0,
        fill: ArcFill::Solid(color),
        segments: ArcSegments::Continuous,
        cap: ArcCap::Butt,
    }
}

/// Ring radius, in percent of the screen's longer side, at which a ring is fully opaque.
const RING_OPAQUE_AT_PCT: u16 = 25;
/// Ring radius, in percent of the screen's longer side, at which a ring has faded out.
const RING_VANISH_AT_PCT: u16 = 50;
/// Time a ring takes to fade in, ending at `RING_OPAQUE_AT_PCT`.
const RING_FADE_IN_MS: NonZeroU16 = NonZeroU16::new(300).expect("BUG: a fade-in takes time");
/// Time from the alarm showing to the first ring starting to fade in.
const RING_HIDDEN_MS: u16 = 1_500;
/// Time a ring takes from starting to fade in to vanishing.
const RING_LOOP_MS: NonZeroU16 = NonZeroU16::new(3_000).expect("BUG: a loop takes time");
const _: () = assert!(
    RING_FADE_IN_MS.get() < RING_LOOP_MS.get(),
    "a ring must fade in before its loop ends"
);
const _: () = assert!(
    RING_OPAQUE_AT_PCT < RING_VANISH_AT_PCT,
    "a ring must turn opaque before it vanishes"
);
const _: () = assert!(
    RING_HIDDEN_MS.checked_add(RING_LOOP_MS.get()).is_some(),
    "the last ring's delay, under a loop past the hidden start, must fit a u16"
);
const RING_COUNT: NonZeroU16 = NonZeroU16::new(4).expect("BUG: the ripple has a ring");
/// Rings spawn evenly through one loop.
#[expect(clippy::integer_division, reason = "precision not needed")]
const RING_SPAWN_INTERVAL_MS: u16 = RING_LOOP_MS.get() / RING_COUNT.get();

/// How a ring's radius grows, and its opacity fades out with it.
const RING_GROWTH: Easing = Easing::EaseOut;
/// How a ring fades in, ending opaque at `RING_OPAQUE_AT_PCT`.
const RING_FADE_IN: Easing = Easing::EaseIn;

/// One ring of the ripple, spawned `spawn_ms` after the first one.
///
/// A ring grows out from where it appears, about a fifth of the screen's longer side,
/// slowing down,
/// transparent until it fades in over `RING_FADE_IN_MS`,
/// opaque at `RING_OPAQUE_AT_PCT`,
/// then fading out to gone by `RING_VANISH_AT_PCT`.
///
/// Each fade overshoots 1 over the loop and the renderer clamps it,
/// so their product is the fade-in, then the fade-out.
/// Until the ring spawns, the fade-in holds its `from` of 0 and hides it.
/// The fade-out shares the growth's easing, which keeps it linear in the radius.
/// `Scale` widens the 1px stroke with the radius.
fn ripple_ring(cx: f32, cy: f32, longer_side: f32, spawn_ms: u16) -> Draw {
    let appear_radius = longer_side * appear_at_pct() / 100.0;
    let start_ms = RING_HIDDEN_MS + spawn_ms;
    draw_outline_circle(cx, cy, appear_radius, ORANGE_40)
        .animate_delayed(
            AnimProperty::Scale,
            1.0,
            f32::from(RING_VANISH_AT_PCT) / appear_at_pct(),
            u32::from(RING_LOOP_MS.get()),
            start_ms,
            RING_GROWTH,
            LoopMode::Forever,
        )
        .animate_delayed(
            AnimProperty::Alpha,
            0.0,
            1.0 / apply_easing(RING_FADE_IN, RING_FADE_IN_SHARE),
            u32::from(RING_LOOP_MS.get()),
            start_ms,
            RING_FADE_IN,
            LoopMode::Forever,
        )
        .animate_delayed(
            AnimProperty::Alpha,
            1.0 / (1.0 - opaque_progress()),
            0.0,
            u32::from(RING_LOOP_MS.get()),
            start_ms,
            RING_GROWTH,
            LoopMode::Forever,
        )
}

/// The share of a loop a ring spends fading in.
const RING_FADE_IN_SHARE: f32 = RING_FADE_IN_MS.get() as f32 / RING_LOOP_MS.get() as f32;

/// How far through its growth a ring is when it turns opaque.
fn opaque_progress() -> f32 {
    apply_easing(RING_GROWTH, RING_FADE_IN_SHARE)
}

/// Ring radius, in percent of the screen's longer side, at which a ring starts fading in:
/// far enough in that `RING_FADE_IN_MS` of growth ends at `RING_OPAQUE_AT_PCT`.
fn appear_at_pct() -> f32 {
    let progress = opaque_progress();
    (f32::from(RING_OPAQUE_AT_PCT) - progress * f32::from(RING_VANISH_AT_PCT)) / (1.0 - progress)
}

fn background_rings(background_size: (u32, u32), motion: RingMotion) -> Node {
    /// `(opacity, radius_px)` of the still rings on the 480×1280 firing screen;
    /// each ring is larger than the 480px width, so only its top and bottom arcs are visible.
    const STILL_RINGS: [(f32, f32); 4] = [(1.0, 350.0), (0.8, 425.0), (0.6, 500.0), (0.4, 575.0)];

    #[expect(clippy::cast_precision_loss)]
    let (w, h) = (background_size.0 as f32, background_size.1 as f32);

    let draws = match motion {
        RingMotion::Ripple => (0..RING_COUNT.get())
            .map(|ring| ripple_ring(w / 2.0, h / 2.0, w.max(h), ring * RING_SPAWN_INTERVAL_MS))
            .collect(),
        RingMotion::Still => STILL_RINGS
            .into_iter()
            .map(|(opacity, r)| {
                draw_outline_circle(w / 2.0, h / 2.0, r, ORANGE_40.with_alpha(opacity))
            })
            .collect(),
    };

    Node::Canvas {
        props: PropsData {
            inset_left: 0.0,
            inset_top: 0.0,
            width: w,
            height: h,
            ..Default::default()
        },
        touch_key: None,
        draws,
    }
}

/// Build the time row: the large time text with an inline AM/PM marker
/// vertically centred against it in 12-hour mode. In 24-hour mode
/// (`view.period` empty) the row is just the time text.
fn build_time_row(view: &AlarmView) -> Node {
    const TIME_FONT_SIZE: u16 = 200;
    let time_node = text(
        view.time.clone(),
        TextStyle {
            size: u32::from(TIME_FONT_SIZE),
            color: WHITE,
            weight: FontWeight::REGULAR,
            align: TextAlign::Center,
            family: FontFamily::DeckSans,
            line_height: 1.0,
            ..Default::default()
        },
    );

    let children = if view.period.is_empty() {
        vec![time_node]
    } else {
        // 12-hour mode: the AM/PM marker sits smaller to the right of the
        // time, vertically centred against it — same arrangement as the
        // clock widget.
        const PERIOD_TEXT_SIZE: u16 = 40;
        const PERIOD_SLOT_W: f32 = PERIOD_TEXT_SIZE as f32 * 2.4;
        const PERIOD_LIFT: f32 = PERIOD_TEXT_SIZE as f32;
        let period_text = text(
            view.period.clone(),
            TextStyle {
                size: u32::from(PERIOD_TEXT_SIZE),
                color: ORANGE_30,
                weight: FontWeight::REGULAR,
                align: TextAlign::Center,
                line_height: 1.0,
                ..Default::default()
            },
        );

        let period_slot = col(
            PropsData {
                width: PERIOD_SLOT_W,
                cross_align: CrossAlign::Start,
                ..Default::default()
            },
            [period_text, fixed_height(PERIOD_LIFT)],
        );
        let left_padding = col(
            PropsData {
                width: PERIOD_SLOT_W,
                ..Default::default()
            },
            [],
        );

        vec![left_padding, time_node, period_slot]
    };

    row(
        PropsData {
            gap: 16.0,
            cross_align: CrossAlign::Center,
            ..Default::default()
        },
        children,
    )
}

#[must_use]
pub fn build_alarm_ui_tree(view: &AlarmView, size: (u32, u32), intl: &Intl) -> Node {
    // Fall back to a generic name for an unlabelled alarm so the screen never
    // shows a blank line above the time.
    let label = if view.label.is_empty() {
        format_message!(
            intl,
            default_message: "Alarm",
            description: "Ringing alarm screen, above the time, when the owner gave the alarm no name",
        )
    } else {
        view.label.clone()
    };
    let label_node = text(
        label,
        TextStyle {
            size: 48,
            color: ORANGE_30,
            weight: FontWeight::REGULAR,
            align: TextAlign::Center,
            family: FontFamily::DeckSans,
            line_height: 1.0,
            ..Default::default()
        },
    );

    let time_row = build_time_row(view);

    let stop = format_message!(
        intl,
        default_message: "Stop Alarm",
        description: "Ringing alarm screen, button that dismisses the alarm",
    );
    let mut buttons = vec![alarm_button(
        STOP_ALARM_KEY,
        &stop,
        Fill::Solid(ORANGE_50),
        None,
    )];
    if view.snooze_allowed {
        let snooze = format_message!(
            intl,
            default_message: "Snooze",
            description: "Ringing alarm screen, button that silences the alarm and rings it again a few minutes later",
        );
        buttons.push(alarm_button(
            SNOOZE_ALARM_KEY,
            &snooze,
            Fill::Solid(TRANSPARENT),
            Some(GRAY_30),
        ));
    }

    let buttons_row = row(
        PropsData {
            gap: 48.0,
            cross_align: CrossAlign::Center,
            ..Default::default()
        },
        buttons,
    );

    col(
        PropsData {
            background: BLACK,
            cross_align: CrossAlign::Center,
            ..Default::default()
        },
        [
            background_rings(size, view.rings),
            fixed_height(62.0),
            label_node,
            fixed_height(32.0),
            time_row,
            buttons_row,
        ],
    )
}

#[derive(Default, Debug)]
pub struct RenderAlarmOutput {
    stop: bool,
    snooze: bool,
    outcome: RenderOutcome,
}

pub fn render_alarm(
    r: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut AlarmRenderState,
    view: &AlarmView,
    intl: &Intl,
    frame: OverlayFrame,
) -> RenderAlarmOutput {
    let tree_frame = match state.tree.render(frame, size, r, intl, || {
        build_alarm_ui_tree(view, size, intl)
    }) {
        Ok(tree_frame) => tree_frame,
        Err(err) => {
            tracing::error!("alarm tree render failed: {err}");
            return RenderAlarmOutput::default();
        }
    };

    let clicks = &tree_frame.result.clicks;
    RenderAlarmOutput {
        stop: clicks.contains_key(STOP_ALARM_KEY),
        snooze: clicks.contains_key(SNOOZE_ALARM_KEY),
        outcome: tree_frame.outcome(),
    }
}

impl Default for AlarmOverlay {
    fn default() -> Self {
        AlarmOverlay::new(Instant::now())
    }
}

impl AlarmOverlay {
    fn new(now: Instant) -> Self {
        Self {
            ringing: None,
            pending: Vec::new(),
            dirty: false,
            render_state: AlarmRenderState::new(now),
            ripple_marker: PathBuf::from(RIPPLE_MARKER),
        }
    }
}

impl SystemOverlay for AlarmOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: "bmc-overlay-alarm".to_owned(),
            input: InputRegion::Full,
        }
    }

    fn uses_alarm(&self) -> bool {
        true
    }

    fn on_alarm_ring(&mut self, time: &str, period: &str, label: &str, snooze_allowed: bool) {
        // Every ring starts the rings from their hidden start, however long the overlay waited
        // and however bmc's stop and ring for a replacing alarm were batched.
        self.render_state.tree.restart();
        self.ringing = Some(Ring {
            time: time.to_owned(),
            period: period.to_owned(),
            label: label.to_owned(),
            snooze_allowed,
            motion: if self.ripple_marker.exists() {
                RingMotion::Ripple
            } else {
                RingMotion::Still
            },
        });
        self.dirty = true;
    }

    fn on_alarm_stop(&mut self) {
        // Stopped from elsewhere (timeout, web UI, or bmc fallback): drop the
        // ringing state so `tick` reports `visible = false` and the framework
        // unmaps the surface.
        self.ringing = None;
    }

    fn tick(&mut self, _now: Instant) -> TickOutcome {
        TickOutcome {
            visible: self.ringing.is_some(),
            wants_render: std::mem::take(&mut self.dirty),
            // Purely event-driven: nothing to poll for, so no timed wake.
            next_wake: None,
        }
    }

    fn on_touch(&mut self, event: TouchEvent) {
        let Some(_ring) = self.ringing.as_ref() else {
            return;
        };

        self.render_state.tree.push_touch(event);
        self.dirty = true;
    }

    fn drain_alarm_requests(&mut self) -> Vec<AlarmRequest> {
        std::mem::take(&mut self.pending)
    }

    fn render(
        &mut self,
        r: &mut dyn Renderer,
        viewport: OverlayViewport,
        intl: &Intl,
        frame: FrameKind,
    ) -> RenderOutcome {
        let Some(ring) = self.ringing.as_ref() else {
            return RenderOutcome::default();
        };

        let view: AlarmView = AlarmView {
            time: ring.time.clone(),
            period: ring.period.clone(),
            label: ring.label.clone(),
            snooze_allowed: ring.snooze_allowed,
            rings: ring.motion,
        };

        let output = render_alarm(
            r,
            viewport.size(),
            &mut self.render_state,
            &view,
            intl,
            OverlayFrame::on_surface(frame),
        );

        if output.stop {
            self.pending.push(AlarmRequest::Dismiss);
        } else if output.snooze {
            self.pending.push(AlarmRequest::Snooze);
        }
        output.outcome
    }

    fn tree_ui(&mut self) -> Option<&mut TreeUi> {
        Some(&mut self.render_state.tree)
    }
}

#[cfg(test)]
mod tests {
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

    /// The `(radius, opacity)` of every ring drawn `elapsed_ms` after the alarm appeared.
    fn drawn_rings_at(elapsed_ms: u64) -> Vec<(f32, f32)> {
        let start = Instant::now();
        let mut state = AlarmRenderState::new(start);
        let frame = OverlayFrame {
            kind: FrameKind::Fresh,
            layering: Layering::Unlayered,
            now: start + Duration::from_millis(elapsed_ms),
        };
        render(&mut state, frame)
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

    fn rings_at(elapsed_ms: u64) -> Vec<(f32, f32)> {
        drawn_rings_at(elapsed_ms)
            .into_iter()
            .filter(|&(_, opacity)| opacity > 0.0)
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
        assert_eq!(rings_at(u64::from(RING_HIDDEN_MS) - 1), vec![]);
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
        assert_near(opacity, 0.25, 0.03, "opacity halfway through the fade-in");
    }

    #[test]
    fn every_ring_shows_once_the_ripple_is_running() {
        assert_eq!(
            rings_at(first_loop_ms(3.1)).len(),
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
}

#[cfg(all(test, target_os = "linux"))]
mod static_layer_gl;
