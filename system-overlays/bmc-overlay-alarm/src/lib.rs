// Copyright (C) 2026  Braiins Systems s.r.o.

//! Full-screen firing-alarm overlay.
//!
//! Maps a full-screen surface while an alarm is ringing (driven by the
//! `deck_alarm_v1` `alarm_ringing` event) and unmaps once the user snoozes or
//! dismisses, or the compositor reports the alarm stopped. Snooze/dismiss taps
//! are sent back over the same protocol.

use std::time::Instant;

use bmc_render::colors::{BLACK, Color, GRAY_30, ORANGE_30, ORANGE_50, WHITE};
use bmc_render::renderer::Renderer;
use bmc_render::tree::{
    DrawCommand, FontFamily, FontWeight, TextAlign, TextStyle, TreeNode, VerticalAlign, col, row,
    text,
};
use bmc_system_overlay::{
    AlarmRequest, Anchor, InputRegion, Layer, LayerConfig, SystemOverlay, TickOutcome, TouchEvent,
    TreeUi,
};
use bmc_wasm_protocol::{CrossAlign, Fill, PathPaint, PropsData, TRANSPARENT};

/// Touch key for the stop-alarm button; shared by its `Canvas` hit region and
/// the `TreeResult::clicks` lookup so `render` and the click handler agree.
const STOP_ALARM_KEY: &str = "stop_alarm_button";
const SNOOZE_ALARM_KEY: &str = "snooze_alarm_button";

/// The firing alarm currently shown, as delivered by the `alarm_ringing` event.
#[derive(Debug)]
struct Ring {
    /// Scheduled alarm time, preformatted for display (e.g. `07:30`).
    time: String,
    /// Alarm label; empty when the alarm has no name, in which case the screen
    /// falls back to "Alarm" (see `build_alarm_ui_tree`).
    label: String,
    /// Whether the alarm may be snoozed; `false` hides the snooze button.
    snooze_allowed: bool,
}

/// Keeps context for UI rendering.
#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct AlarmRenderState {
    tree: TreeUi,
    last_render: Instant,
}

impl AlarmRenderState {
    #[must_use]
    pub fn new(now: Instant) -> AlarmRenderState {
        Self {
            tree: TreeUi::new(),
            last_render: now,
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
}

#[derive(Debug, Clone)]
pub struct AlarmView {
    pub time: String,
    /// Alarm label; empty falls back to "Alarm" at render (see `build_alarm_ui_tree`).
    pub label: String,
    /// Whether the snooze button is shown; `false` renders stop-only.
    pub snooze_allowed: bool,
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
fn alarm_button(key: &str, label: &str, fill: Fill, border: Option<Color>) -> TreeNode {
    /// Logical size of every alarm action button.
    const BUTTON_W: f32 = 400.0;
    const BUTTON_H: f32 = 100.0;

    let mut draws = vec![DrawCommand::Rect {
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
        draws.push(DrawCommand::Path {
            points: vec![
                (0.5, 0.5),
                (BUTTON_W - 0.5, 0.5),
                (BUTTON_W - 0.5, BUTTON_H - 0.5),
                (0.5, BUTTON_H - 0.5),
            ],
            paint: PathPaint::Stroke { color, width: 1.0 },
            closed: true,
            smooth: false,
        });
    }

    // Placed at the canvas centre; `align`/`vertical_align: Center` anchor the
    // glyph box on that point rather than its top-left. Pushed last so the
    // label paints on top of the fill and border.
    draws.push(DrawCommand::Text {
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

    TreeNode::Canvas {
        props: PropsData {
            width: BUTTON_W,
            height: BUTTON_H,
            ..Default::default()
        },
        touch_key: Some(key.to_owned()),
        draws,
    }
}

/// A fixed vertical gap: an empty column of the given height. The layout
/// engine has only uniform `padding`/`margin`, so top-only spacing is added as
/// a sized spacer between siblings.
fn fixed_height(height: f32) -> TreeNode {
    col(
        PropsData {
            height,
            ..Default::default()
        },
        Vec::new(),
    )
}

#[must_use]
pub fn build_alarm_ui_tree(view: &AlarmView, size: (u32, u32)) -> TreeNode {
    // Fall back to a generic name for an unlabelled alarm so the screen never
    // shows a blank line above the time.
    let label = if view.label.is_empty() {
        "Alarm"
    } else {
        view.label.as_str()
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

    let time_node = text(
        view.time.clone(),
        TextStyle {
            size: 200,
            color: WHITE,
            weight: FontWeight::REGULAR,
            align: TextAlign::Center,
            family: FontFamily::DeckSans,
            line_height: 1.0,
            ..Default::default()
        },
    );

    let mut buttons = vec![alarm_button(
        STOP_ALARM_KEY,
        "Stop Alarm",
        Fill::Solid(ORANGE_50),
        None,
    )];
    if view.snooze_allowed {
        buttons.push(alarm_button(
            SNOOZE_ALARM_KEY,
            "Snooze",
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
            fixed_height(62.0),
            label_node,
            fixed_height(32.0),
            time_node,
            buttons_row,
        ],
    )
}

#[derive(Default, Debug)]
pub struct RenderAlarmOutput {
    stop: bool,
    snooze: bool,
}

pub fn render_alarm(
    r: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut AlarmRenderState,
    view: &AlarmView,
) -> RenderAlarmOutput {
    let now = Instant::now();
    let delta_ms = u32::try_from(now.saturating_duration_since(state.last_render).as_millis())
        .unwrap_or(u32::MAX);
    state.last_render = now;

    let tree = build_alarm_ui_tree(view);

    let result = match state.tree.render(&tree, size, delta_ms, r) {
        Ok(result) => result,
        Err(err) => {
            tracing::error!("alarm tree render failed: {err}");
            return RenderAlarmOutput::default();
        }
    };

    RenderAlarmOutput {
        stop: result.clicks.contains_key(STOP_ALARM_KEY),
        snooze: result.clicks.contains_key(SNOOZE_ALARM_KEY),
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

    fn on_alarm_ring(&mut self, time: &str, label: &str, snooze_allowed: bool) {
        self.ringing = Some(Ring {
            time: time.to_owned(),
            label: label.to_owned(),
            snooze_allowed,
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

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        let Some(ring) = self.ringing.as_ref() else {
            return;
        };

        let view: AlarmView = AlarmView {
            time: ring.time.clone(),
            label: ring.label.clone(),
            snooze_allowed: ring.snooze_allowed,
        };

        let output = render_alarm(r, size, &mut self.render_state, &view);

        if output.stop {
            self.pending.push(AlarmRequest::Dismiss);
        } else if output.snooze {
            self.pending.push(AlarmRequest::Snooze);
        }
    }
}
