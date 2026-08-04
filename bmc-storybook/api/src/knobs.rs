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

//! Story context with interactive knobs (controls panel).
//!
//! Knobs are "declarative by use" — calling `ctx.slider(...)` both reads the
//! current value and registers the control. On the first frame, knobs are
//! created with their defaults. On subsequent frames, the current
//! (possibly user-modified) values are returned.
//!
//! Each knob method returns a typed **handle** that carries the current value
//! and the knob's identity (index). Handles produce [`Nudge`] values for
//! use with [`StoryCtx::bind`], enabling preview interactions (clicks, drags)
//! to back-propagate into the Controls panel.

use bmc_render::colors::Color;

use crate::{DocBlock, StoryUi};

// ── Knob storage ─────────────────────────────────────────────────────

/// A single interactive control in the knobs panel.
///
/// Public so the shell can render knob UI (egui lives in the shell, not here).
#[derive(Debug, Clone)]
pub enum Knob {
    Text {
        label: String,
        value: String,
    },
    Slider {
        label: String,
        value: f32,
        min: f32,
        max: f32,
        /// Snap increment; 0.0 for a continuous slider.
        step: f32,
    },
    Toggle {
        label: String,
        value: bool,
    },
    Color {
        label: String,
        value: Color,
    },
    Select {
        label: String,
        value: usize,
        options: Vec<String>,
        radio: bool,
    },
    /// 2-axis touchpad — bundles two scalar values (e.g. lat/lon, pitch/yaw)
    /// into a single drag-target so the controls panel doesn't need two
    /// sliders for orthogonal angles.
    ///
    /// `invert_y` flips the screen-Y → value-Y mapping: when `true`, the
    /// pointer at the top of the pad maps to `max_y` (Blender-style camera
    /// pads). When `false`, screen-Y matches axis-Y directly (top = `min_y`),
    /// which is the unsurprising read for an absolute-position control.
    Pad2D {
        label: String,
        x: f32,
        y: f32,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
        invert_y: bool,
    },
    /// Labeled group separator — visually groups subsequent knobs.
    Group {
        label: String,
    },
}

impl Knob {
    /// The label shared by all knob variants.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Knob::Text { label, .. }
            | Knob::Slider { label, .. }
            | Knob::Toggle { label, .. }
            | Knob::Color { label, .. }
            | Knob::Select { label, .. }
            | Knob::Pad2D { label, .. }
            | Knob::Group { label } => label,
        }
    }

    /// Copy the value from `other` if it's the same variant. No-op on mismatch.
    fn restore_value_from(&mut self, other: &Knob) {
        match (self, other) {
            (Knob::Slider { value, .. }, Knob::Slider { value: v, .. }) => *value = *v,
            (Knob::Toggle { value, .. }, Knob::Toggle { value: v, .. }) => *value = *v,
            (Knob::Text { value, .. }, Knob::Text { value: v, .. }) => value.clone_from(v),
            (Knob::Color { value, .. }, Knob::Color { value: v, .. }) => *value = *v,
            (Knob::Select { value, .. }, Knob::Select { value: v, .. }) => *value = *v,
            (Knob::Pad2D { x, y, .. }, Knob::Pad2D { x: ox, y: oy, .. }) => {
                *x = *ox;
                *y = *oy;
            }
            _ => {}
        }
    }
}

// ── Knob handles ─────────────────────────────────────────────────────

/// Handle to a slider knob. Carries the current value and knob identity.
#[derive(Debug, Clone, Copy)]
pub struct SliderKnob {
    index: usize,
    value: f32,
    min: f32,
    max: f32,
}

impl SliderKnob {
    #[must_use]
    pub fn get(self) -> f32 {
        self.value
    }

    #[doc(hidden)]
    #[must_use]
    pub fn index(self) -> usize {
        self.index
    }

    #[doc(hidden)]
    #[must_use]
    pub fn min(self) -> f32 {
        self.min
    }

    #[doc(hidden)]
    #[must_use]
    pub fn max(self) -> f32 {
        self.max
    }

    /// Get the current value as `i32` (truncates toward zero).
    #[must_use]
    #[expect(clippy::cast_possible_truncation)]
    pub fn get_i32(self) -> i32 {
        self.value as i32
    }

    /// Get the current value as `usize` (clamps to zero).
    #[must_use]
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn get_usize(self) -> usize {
        self.value.max(0.0) as usize
    }

    /// Create a [`Nudge`] that adds `delta` to this slider (clamped to its range).
    #[must_use]
    pub fn nudge(self, delta: f32) -> Nudge {
        Nudge::SliderNudge {
            index: self.index,
            delta,
        }
    }

    /// Create a [`Nudge`] that sets this slider to an exact value (clamped).
    #[must_use]
    pub fn set(self, value: f32) -> Nudge {
        Nudge::SliderSet {
            index: self.index,
            value,
        }
    }
}

impl From<SliderKnob> for f32 {
    fn from(knob: SliderKnob) -> Self {
        knob.value
    }
}

impl From<SliderKnob> for i32 {
    #[expect(clippy::cast_possible_truncation)]
    fn from(knob: SliderKnob) -> Self {
        knob.value as Self
    }
}

impl From<&SliderKnob> for i32 {
    #[expect(clippy::cast_possible_truncation)]
    fn from(knob: &SliderKnob) -> Self {
        knob.value as Self
    }
}

impl From<SliderKnob> for usize {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from(knob: SliderKnob) -> Self {
        knob.value.max(0.0) as Self
    }
}

impl From<&SliderKnob> for usize {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from(knob: &SliderKnob) -> Self {
        knob.value.max(0.0) as Self
    }
}

impl From<&SliderKnob> for f32 {
    fn from(knob: &SliderKnob) -> Self {
        knob.value
    }
}

/// Handle to a toggle knob. Carries the current value and knob identity.
#[derive(Debug, Clone, Copy)]
pub struct ToggleKnob {
    index: usize,
    value: bool,
}

impl ToggleKnob {
    #[must_use]
    pub fn get(self) -> bool {
        self.value
    }

    /// Create a [`Nudge`] that flips this toggle.
    #[must_use]
    pub fn flip(self) -> Nudge {
        Nudge::ToggleFlip { index: self.index }
    }

    /// Create a [`Nudge`] that sets this toggle to an exact value.
    #[must_use]
    pub fn set(self, value: bool) -> Nudge {
        Nudge::ToggleSet {
            index: self.index,
            value,
        }
    }
}

impl From<ToggleKnob> for bool {
    fn from(knob: ToggleKnob) -> Self {
        knob.value
    }
}

/// Handle to a text knob. Carries the current value and knob identity.
#[derive(Debug, Clone)]
pub struct TextKnob {
    index: usize,
    value: String,
}

impl TextKnob {
    #[must_use]
    pub fn get(&self) -> &str {
        &self.value
    }

    /// Create a [`Nudge`] that sets this text to an exact value.
    #[must_use]
    pub fn set(&self, value: impl Into<String>) -> Nudge {
        Nudge::TextSet {
            index: self.index,
            value: value.into(),
        }
    }
}

impl AsRef<str> for TextKnob {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl From<TextKnob> for String {
    fn from(knob: TextKnob) -> Self {
        knob.value
    }
}

impl From<&TextKnob> for String {
    fn from(knob: &TextKnob) -> Self {
        knob.value.clone()
    }
}

/// Handle to a color knob. Carries the current value and knob identity.
#[derive(Debug, Clone, Copy)]
pub struct ColorKnob {
    index: usize,
    value: Color,
}

impl ColorKnob {
    #[must_use]
    pub fn get(self) -> Color {
        self.value
    }

    /// Create a [`Nudge`] that sets this color to an exact value.
    #[must_use]
    pub fn set(self, value: Color) -> Nudge {
        Nudge::ColorSet {
            index: self.index,
            value,
        }
    }
}

impl From<ColorKnob> for Color {
    fn from(knob: ColorKnob) -> Self {
        knob.value
    }
}

/// Configuration for a [`Pad2DKnob`] registration.
///
/// Defaults: pad covers `[-1, 1]²` with both axes centred at zero, no Y
/// inversion. Override only the fields you care about and use
/// `..Default::default()` for the rest, matching the `ModalProps` pattern
/// used elsewhere in the SDK.
#[derive(Debug, Clone)]
pub struct Pad2DSpec {
    pub x: f32,
    pub y: f32,
    pub range_x: std::ops::RangeInclusive<f32>,
    pub range_y: std::ops::RangeInclusive<f32>,
    /// When `true`, screen-Y → value-Y is flipped: pointer at the top of
    /// the pad maps to `*range_y.end()` (Blender-style orientation pads).
    /// When `false` (default), top of pad maps to `*range_y.start()`,
    /// which reads as an absolute-position control like a 2-D map.
    pub invert_y: bool,
}

impl Default for Pad2DSpec {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            range_x: -1.0..=1.0,
            range_y: -1.0..=1.0,
            invert_y: false,
        }
    }
}

/// Handle to a 2-axis touchpad knob. Carries the current `(x, y)` and the
/// declared ranges for value-from-position mapping in the renderer.
#[derive(Debug, Clone, Copy)]
pub struct Pad2DKnob {
    index: usize,
    x: f32,
    y: f32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    invert_y: bool,
}

impl Pad2DKnob {
    /// Current `(x, y)` value as a tuple — convenient when both are needed.
    #[must_use]
    pub fn get(self) -> (f32, f32) {
        (self.x, self.y)
    }

    #[must_use]
    pub fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub fn y(self) -> f32 {
        self.y
    }

    #[doc(hidden)]
    #[must_use]
    pub fn index(self) -> usize {
        self.index
    }

    #[doc(hidden)]
    #[must_use]
    pub fn ranges(self) -> (f32, f32, f32, f32) {
        (self.min_x, self.max_x, self.min_y, self.max_y)
    }

    /// Whether the renderer should flip screen-Y → value-Y mapping.
    #[doc(hidden)]
    #[must_use]
    pub fn invert_y(self) -> bool {
        self.invert_y
    }
}

/// Handle to a select knob (radio group). Carries the selected index.
#[derive(Debug, Clone, Copy)]
pub struct SelectKnob {
    value: usize,
}

impl SelectKnob {
    /// Get the selected option index.
    #[must_use]
    pub fn get(self) -> usize {
        self.value
    }
}

impl From<SelectKnob> for usize {
    fn from(knob: SelectKnob) -> Self {
        knob.value
    }
}

impl From<&SelectKnob> for usize {
    fn from(knob: &SelectKnob) -> Self {
        knob.value
    }
}

// ── Nudge ────────────────────────────────────────────────────────────

/// A declarative mutation to apply to a knob.
///
/// Created by handle methods (e.g. [`SliderKnob::nudge`]), consumed by
/// [`StoryCtx::bind`] to wire preview interactions to knob state changes.
#[derive(Debug, Clone)]
pub enum Nudge {
    SliderNudge { index: usize, delta: f32 },
    SliderSet { index: usize, value: f32 },
    ToggleFlip { index: usize },
    ToggleSet { index: usize, value: bool },
    TextSet { index: usize, value: String },
    ColorSet { index: usize, value: Color },
}

// ── Story context ────────────────────────────────────────────────────

/// A registered action: stable element key + human-readable label shown in
/// the Actions panel.
#[derive(Clone, Debug)]
pub struct Action {
    pub key: String,
    pub label: String,
}

/// An action that fired last frame: original element-key prefix + the
/// suffix portion of the actual fired key (e.g. `_plus`, `_minus`).
#[derive(Clone, Debug)]
struct FiredAction {
    key: String,
    sub: String,
}

/// Action-to-knob binding: when `action_key` fires with suffix `sub`, apply
/// `nudge` to the bound knob before the next frame's story function runs.
#[derive(Clone, Debug)]
struct Binding {
    action_key: String,
    sub: String,
    nudge: Nudge,
}

/// Drag-to-slider binding: while the user drags on `action_key`, set the
/// slider at `knob_idx` to `min + fraction * (max - min)`.
#[derive(Clone, Debug)]
struct DragBinding {
    action_key: String,
    knob_idx: usize,
    min: f32,
    max: f32,
}

/// Context passed to story render functions.
///
/// Provides knob registration/read API, action registration, and
/// interaction binding for back-propagation from the preview into controls.
#[derive(Debug)]
pub struct StoryCtx {
    knobs: Vec<Knob>,
    cursor: usize,
    /// Registered actions, indexed by `action_cursor` order.
    actions: Vec<Action>,
    action_cursor: usize,
    /// Bindings applied on `begin_frame` when matching fired actions are found.
    bindings: Vec<Binding>,
    /// Actions that fired last frame — readable by the story this frame.
    fired_actions: Vec<FiredAction>,
    /// Actions that fired this frame — swapped into `fired_actions` on next `begin_frame`.
    fired_actions_pending: Vec<FiredAction>,
    /// Drag bindings active for this frame.
    drag_bindings: Vec<DragBinding>,
    /// Knob values from the previous .so load, waiting to be restored after the
    /// first story render re-registers all knobs. Matched by (label, type).
    pending_restore: Option<Vec<Knob>>,
    /// Document block builder — stories push frames, headers, code blocks, etc.
    pub ui: StoryUi,
}

impl StoryCtx {
    #[must_use]
    pub fn new() -> Self {
        Self {
            knobs: Vec::new(),
            cursor: 0,
            actions: Vec::new(),
            action_cursor: 0,
            bindings: Vec::new(),
            fired_actions: Vec::new(),
            fired_actions_pending: Vec::new(),
            drag_bindings: Vec::new(),
            pending_restore: None,
            ui: StoryUi::new(),
        }
    }

    /// Create a new context that will restore knob values from `old_knobs`
    /// after the first story render re-registers them.
    #[must_use]
    pub fn new_with_restore(old_knobs: Vec<Knob>) -> Self {
        let mut ctx = Self::new();
        if !old_knobs.is_empty() {
            ctx.pending_restore = Some(old_knobs);
        }
        ctx
    }

    /// After the story's first render, match newly registered knobs against
    /// the snapshot by (label, type) and restore previous values.
    pub fn apply_pending_restore(&mut self) {
        let Some(old) = self.pending_restore.take() else {
            return;
        };
        for knob in &mut self.knobs {
            let label = knob.label();
            if let Some(prev) = old.iter().find(|k| k.label() == label) {
                knob.restore_value_from(prev);
            }
        }
    }

    /// Reset cursors for a new frame (keeps knob values).
    pub fn begin_frame(&mut self) {
        // Swap: pending results from last render become readable this frame.
        std::mem::swap(&mut self.fired_actions, &mut self.fired_actions_pending);
        self.fired_actions_pending.clear();

        // Apply bindings (from previous frame) against the newly available fired actions.
        self.apply_bindings();

        // Truncate the knob list to the previous render's final cursor — drops
        // stale entries left behind by stories that shorten their conditional
        // control layout (e.g. hiding a knob when a checkbox is off). Without
        // this the controls panel keeps rendering the old knob and value
        // accessors mismatch by index after a layout-shrinking re-render.
        self.knobs.truncate(self.cursor);

        // Reset cursors.
        self.cursor = 0;
        self.action_cursor = 0;
        self.bindings.clear();
        self.drag_bindings.clear();
        self.ui.clear();
    }

    /// Drain document blocks emitted by the story's render function.
    pub fn take_doc_blocks(&mut self) -> Vec<DocBlock> {
        self.ui.take_blocks()
    }

    // ── Actions ──────────────────────────────────────────────────────

    /// Register a named action for the Actions panel.
    ///
    /// Returns a stable element key. Use it as the interaction ID in your
    /// widget tree (e.g. the first argument to `button!`). When the element
    /// is clicked, the action name appears in the Actions panel.
    ///
    /// ```ignore
    /// let on_click = ctx.action("Primary clicked");
    /// button!(&on_click, "Click me", style: Primary)
    /// ```
    pub fn action(&mut self, name: &str) -> String {
        let idx = self.action_cursor;
        self.action_cursor += 1;
        let key = format!("__action_{idx}_");
        if idx >= self.actions.len() {
            self.actions.push(Action {
                key: key.clone(),
                label: name.to_owned(),
            });
        }
        key
    }

    /// Register a named action with a specific interaction key.
    ///
    /// Use this when the widget uses hardcoded button keys (e.g. `"kb::cancel"`)
    /// rather than keys generated by [`action`](Self::action).
    pub fn action_with_key(&mut self, name: &str, key: &str) {
        self.action_cursor += 1;
        if !self.actions.iter().any(|a| a.key == key) {
            self.actions.push(Action {
                key: key.to_owned(),
                label: name.to_owned(),
            });
        }
    }

    /// Get all registered action mappings.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Record that an action fired (called by the app after render).
    pub fn record_fired_action(&mut self, key: String, sub: String) {
        self.fired_actions_pending.push(FiredAction { key, sub });
    }

    // ── Bindings ─────────────────────────────────────────────────────

    /// Bind an action sub-event to a knob mutation.
    ///
    /// When the action identified by `action_key` fires with suffix `sub`,
    /// the `nudge` is applied to the target knob before the next frame's
    /// story function runs.
    ///
    /// ```ignore
    /// let value = ctx.slider("Temperature", 25.0, 0.0, 100.0);
    /// let key = ctx.action("Temperature changed");
    /// ctx.bind(&key, "_plus",  value.nudge(1.0));
    /// ctx.bind(&key, "_minus", value.nudge(-1.0));
    /// ```
    pub fn bind(&mut self, action_key: &str, sub: &str, nudge: Nudge) {
        self.bindings.push(Binding {
            action_key: action_key.to_owned(),
            sub: sub.to_owned(),
            nudge,
        });
    }

    /// Bind a drag gesture on `action_key` to a slider knob.
    ///
    /// While the user drags on the element registered with this action key,
    /// the slider's value is set to `min + fraction * (max - min)`, where
    /// `fraction` is the horizontal drag position normalized to `[0, 1]`.
    ///
    /// ```ignore
    /// let fraction = ctx.slider("Progress", 0.6, 0.0, 1.0);
    /// let key = ctx.action("Drag");
    /// ctx.bind_drag(&key, fraction);
    /// progress_bar!(ProgressMode::Slider(fraction.get()), touch_key: &key)
    /// ```
    pub fn bind_drag(&mut self, action_key: &str, slider: SliderKnob) {
        self.drag_bindings.push(DragBinding {
            action_key: action_key.to_owned(),
            knob_idx: slider.index(),
            min: slider.min(),
            max: slider.max(),
        });
    }

    /// Apply active drag positions to bound slider knobs.
    ///
    /// Called by the shell after render with the `TreeResult.drags` map.
    /// `drags` maps action keys to `(local_x, element_width)`.
    pub fn apply_drags(&mut self, drags: &[(String, f32, f32)]) {
        for (drag_key, x, width) in drags {
            let fraction = if *width > 0.0 {
                (x / width).clamp(0.0, 1.0)
            } else {
                0.0
            };
            for db in &self.drag_bindings {
                if drag_key == &db.action_key {
                    let value = db.min + fraction * (db.max - db.min);
                    if let Some(Knob::Slider {
                        value: v,
                        min: lo,
                        max: hi,
                        ..
                    }) = self.knobs.get_mut(db.knob_idx)
                    {
                        *v = value.clamp(*lo, *hi);
                    }
                }
            }
        }
    }

    fn apply_bindings(&mut self) {
        // Collect matching nudges first to avoid borrow conflict.
        let nudges: Vec<Nudge> = self
            .bindings
            .iter()
            .filter(|b| {
                self.fired_actions
                    .iter()
                    .any(|fa| fa.key == b.action_key && fa.sub == b.sub)
            })
            .map(|b| b.nudge.clone())
            .collect();

        for nudge in nudges {
            self.apply_nudge(&nudge);
        }
    }

    fn apply_nudge(&mut self, nudge: &Nudge) {
        match nudge {
            Nudge::SliderNudge { index, delta } => {
                if let Some(Knob::Slider {
                    value, min, max, ..
                }) = self.knobs.get_mut(*index)
                {
                    *value = (*value + delta).clamp(*min, *max);
                }
            }
            Nudge::SliderSet {
                index,
                value: new_val,
            } => {
                if let Some(Knob::Slider {
                    value, min, max, ..
                }) = self.knobs.get_mut(*index)
                {
                    *value = new_val.clamp(*min, *max);
                }
            }
            Nudge::ToggleFlip { index } => {
                if let Some(Knob::Toggle { value, .. }) = self.knobs.get_mut(*index) {
                    *value = !*value;
                }
            }
            Nudge::ToggleSet {
                index,
                value: new_val,
            } => {
                if let Some(Knob::Toggle { value, .. }) = self.knobs.get_mut(*index) {
                    *value = *new_val;
                }
            }
            Nudge::TextSet {
                index,
                value: new_val,
            } => {
                if let Some(Knob::Text { value, .. }) = self.knobs.get_mut(*index) {
                    value.clone_from(new_val);
                }
            }
            Nudge::ColorSet {
                index,
                value: new_val,
            } => {
                if let Some(Knob::Color { value, .. }) = self.knobs.get_mut(*index) {
                    *value = *new_val;
                }
            }
        }
    }

    // ── Knobs ────────────────────────────────────────────────────────

    /// Insert a labeled group separator. Visually groups subsequent knobs
    /// under a heading in the Controls panel.
    pub fn group(&mut self, label: &str) {
        let idx = self.cursor;
        self.cursor += 1;
        if idx < self.knobs.len() {
            return;
        }
        self.knobs.push(Knob::Group {
            label: label.to_owned(),
        });
    }

    /// Text input knob.
    pub fn text(&mut self, label: &str, default: &str) -> TextKnob {
        let idx = self.cursor;
        self.cursor += 1;
        if let Some(Knob::Text {
            label: existing,
            value,
        }) = self.knobs.get(idx)
            && existing == label
        {
            return TextKnob {
                index: idx,
                value: value.clone(),
            };
        }
        let value = default.to_owned();
        let new_knob = Knob::Text {
            label: label.to_owned(),
            value: value.clone(),
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        TextKnob { index: idx, value }
    }

    /// Slider knob (continuous f32 value).
    pub fn slider(&mut self, label: &str, default: f32, min: f32, max: f32) -> SliderKnob {
        self.slider_with_step(label, default, min, max, 0.0)
    }

    /// Integer slider knob — snaps to whole numbers,
    /// so the value and its displayed label stay integral.
    pub fn int_slider(&mut self, label: &str, default: f32, min: f32, max: f32) -> SliderKnob {
        self.slider_with_step(label, default, min, max, 1.0)
    }

    fn slider_with_step(
        &mut self,
        label: &str,
        default: f32,
        min: f32,
        max: f32,
        step: f32,
    ) -> SliderKnob {
        let idx = self.cursor;
        self.cursor += 1;
        if let Some(Knob::Slider {
            label: existing,
            value,
            ..
        }) = self.knobs.get(idx)
            && existing == label
        {
            return SliderKnob {
                index: idx,
                value: *value,
                min,
                max,
            };
        }
        let new_knob = Knob::Slider {
            label: label.to_owned(),
            value: default,
            min,
            max,
            step,
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        SliderKnob {
            index: idx,
            value: default,
            min,
            max,
        }
    }

    /// Boolean toggle knob.
    pub fn toggle(&mut self, label: &str, default: bool) -> ToggleKnob {
        let idx = self.cursor;
        self.cursor += 1;
        if let Some(Knob::Toggle {
            label: existing,
            value,
        }) = self.knobs.get(idx)
            && existing == label
        {
            return ToggleKnob {
                index: idx,
                value: *value,
            };
        }
        let new_knob = Knob::Toggle {
            label: label.to_owned(),
            value: default,
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        ToggleKnob {
            index: idx,
            value: default,
        }
    }

    /// Color picker knob.
    pub fn color(&mut self, label: &str, default: Color) -> ColorKnob {
        let idx = self.cursor;
        self.cursor += 1;
        if let Some(Knob::Color {
            label: existing,
            value,
        }) = self.knobs.get(idx)
            && existing == label
        {
            return ColorKnob {
                index: idx,
                value: *value,
            };
        }
        let new_knob = Knob::Color {
            label: label.to_owned(),
            value: default,
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        ColorKnob {
            index: idx,
            value: default,
        }
    }

    /// 2-axis touchpad knob — returns a handle whose `(x, y)` reflect the
    /// current pointer position within the pad. See [`Pad2DSpec`] for the
    /// available configuration; defaults give a `[-1, 1]²` pad with no
    /// inversion.
    pub fn pad2d(&mut self, label: &str, spec: Pad2DSpec) -> Pad2DKnob {
        let idx = self.cursor;
        self.cursor += 1;
        let Pad2DSpec {
            x: default_x,
            y: default_y,
            range_x,
            range_y,
            invert_y,
        } = spec;
        let (min_x, max_x) = (*range_x.start(), *range_x.end());
        let (min_y, max_y) = (*range_y.start(), *range_y.end());

        if let Some(Knob::Pad2D {
            label: existing,
            x,
            y,
            invert_y: existing_invert,
            ..
        }) = self.knobs.get(idx)
            && existing == label
            && *existing_invert == invert_y
        {
            return Pad2DKnob {
                index: idx,
                x: *x,
                y: *y,
                min_x,
                max_x,
                min_y,
                max_y,
                invert_y,
            };
        }
        let new_knob = Knob::Pad2D {
            label: label.to_owned(),
            x: default_x,
            y: default_y,
            min_x,
            max_x,
            min_y,
            max_y,
            invert_y,
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        Pad2DKnob {
            index: idx,
            x: default_x,
            y: default_y,
            min_x,
            max_x,
            min_y,
            max_y,
            invert_y,
        }
    }

    /// Select knob (dropdown combo box).
    pub fn select(&mut self, label: &str, options: &[&str], default: usize) -> SelectKnob {
        self.select_inner(label, options, default, false)
    }

    /// Select knob displayed as a radio button group.
    pub fn radio(&mut self, label: &str, options: &[&str], default: usize) -> SelectKnob {
        self.select_inner(label, options, default, true)
    }

    fn select_inner(
        &mut self,
        label: &str,
        options: &[&str],
        default: usize,
        radio: bool,
    ) -> SelectKnob {
        let idx = self.cursor;
        self.cursor += 1;
        // Reuse only when label, options and presentation all match — a
        // change in any of those means the story author replaced this slot
        // with a different conceptual control and the stored index would
        // map to the wrong option semantically.
        if let Some(Knob::Select {
            label: existing_label,
            value,
            options: existing_options,
            radio: existing_radio,
        }) = self.knobs.get(idx)
            && existing_label == label
            && *existing_radio == radio
            && existing_options.len() == options.len()
            && existing_options
                .iter()
                .zip(options.iter())
                .all(|(a, b)| a == b)
        {
            return SelectKnob { value: *value };
        }
        let new_knob = Knob::Select {
            label: label.to_owned(),
            value: default,
            options: options.iter().map(|s| (*s).to_owned()).collect(),
            radio,
        };
        if idx < self.knobs.len() {
            self.knobs[idx] = new_knob;
        } else {
            self.knobs.push(new_knob);
        }
        SelectKnob { value: default }
    }

    // ── Assets ───────────────────────────────────────────────────────

    // ── Knob access for shell UI ─────────────────────────────────────

    /// Mutable access to the knob list, for the shell's egui-based knob renderer.
    pub fn knobs_mut(&mut self) -> &mut Vec<Knob> {
        &mut self.knobs
    }
}

impl Default for StoryCtx {
    fn default() -> Self {
        Self::new()
    }
}
