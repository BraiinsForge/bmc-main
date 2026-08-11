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

#![allow(
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

//! Pomodoro timer widget with LED integration.
//!
//! Standard 25/5/15 pomodoro technique with configurable durations,
//! LED feedback per phase, and daily session tracking via KV persistence.

use std::cell::Cell;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

// ── Assets ───────────────────────────────────────────────────────────

const CHIME_WORK: Audio = include_audio!("assets/sounds/chime_work.wav");
const CHIME_BREAK: Audio = include_audio!("assets/sounds/chime_break.wav");
const CHIME_DONE: Audio = include_audio!("assets/sounds/chime_done.wav");

const ICON_PLAY: Svg = include_svg!("assets/icons/start.svg");
const ICON_PAUSE: Svg = include_svg!("assets/icons/pause.svg");
const ICON_STOP: Svg = include_svg!("assets/icons/stop.svg");
const ICON_SETTINGS: Svg = include_svg!("assets/icons/settings.svg");

// ── Constants ────────────────────────────────────────────────────────

const SESSIONS_PER_CYCLE: u32 = 4;

// ── State ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Phase {
    Idle = 0,
    Working = 1,
    ShortBreak = 2,
    LongBreak = 3,
}

thread_local! {
    static PHASE: Cell<Phase> = const { Cell::new(Phase::Idle) };
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    static ELAPSED_MS: Cell<u32> = const { Cell::new(0) };
    /// Sessions completed in the current 4-session cycle (0–3).
    static SESSION_COUNT: Cell<u32> = const { Cell::new(0) };
    /// Total pomodoros completed today.
    static TOTAL_COMPLETED: Cell<u32> = const { Cell::new(0) };
    /// Work duration in minutes (default 25).
    static WORK_MIN: Cell<u32> = const { Cell::new(25) };
    /// Short break duration in minutes (default 5).
    static SHORT_BREAK_MIN: Cell<u32> = const { Cell::new(5) };
    /// Long break duration in minutes (default 15).
    static LONG_BREAK_MIN: Cell<u32> = const { Cell::new(15) };
    /// Whether the settings modal is open.
    static SETTINGS_OPEN: Cell<bool> = const { Cell::new(false) };
    /// Date string (YYYY-MM-DD) for daily reset.
    static SAVED_DATE: Cell<[u8; 10]> = const { Cell::new([0; 10]) };
}

// ── Persistence ──────────────────────────────────────────────────────

fn load_u32(key: &str, default: u32) -> u32 {
    kv::get_string(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn save_u32(key: &str, val: u32) {
    kv::set(key, fmt!("{val}").as_bytes());
}

fn today_string() -> [u8; 10] {
    let now = SystemTime::now();
    let local = system::current()
        .timezone()
        .and_then(|name| now.local(&Tz::from_runtime(name)))
        .unwrap_or_else(|| now.utc());
    let mut buf = [0u8; 10];
    // Manual zero-padded date: YYYY-MM-DD
    let year = u32::from(local.year);
    let month = u32::from(local.month);
    let day = u32::from(local.day);
    let s = fmt!(
        "{}{}{}{}-{}{}-{}{}",
        year / 1_000,
        (year / 100) % 10,
        (year / 10) % 10,
        year % 10,
        month / 10,
        month % 10,
        day / 10,
        day % 10
    );
    let bytes = s.as_bytes();
    let len = bytes.len().min(10);
    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

fn load_persisted_state() {
    WORK_MIN.set(load_u32("pomodoro_work_min", 25).clamp(1, 60));
    SHORT_BREAK_MIN.set(load_u32("pomodoro_short_break_min", 5).clamp(1, 30));
    LONG_BREAK_MIN.set(load_u32("pomodoro_long_break_min", 15).clamp(1, 60));

    let today = today_string();
    let saved = kv::get_string("pomodoro_date").unwrap_or_default();
    if saved.as_bytes() == &today[..] {
        TOTAL_COMPLETED.set(load_u32("pomodoro_total", 0));
    } else {
        // New day — reset count
        TOTAL_COMPLETED.set(0);
        kv::set("pomodoro_date", &today);
        save_u32("pomodoro_total", 0);
    }
    SAVED_DATE.set(today);
}

fn persist_total() {
    save_u32("pomodoro_total", TOTAL_COMPLETED.get());
    kv::set("pomodoro_date", &today_string());
}

// ── LED helpers ──────────────────────────────────────────────────────

fn apply_led_for_phase(phase: Phase) {
    match phase {
        Phase::Working => {
            led::set_effect(LedEffect::Breathe, RED_50, 4_000, None);
        }
        Phase::ShortBreak => {
            led::set_effect(LedEffect::Solid, GREEN_50, 0, None);
        }
        Phase::LongBreak => {
            led::set_effect(LedEffect::Solid, BLUE_50, 0, None);
        }
        Phase::Idle => {
            led::stop();
        }
    }
}

fn led_session_complete() {
    led::set_effect(LedEffect::Chase, GREEN_50, 500, Some(2_000));
}

// ── Phase duration ───────────────────────────────────────────────────

fn phase_duration_ms(phase: Phase) -> u32 {
    let min = match phase {
        Phase::Working => WORK_MIN.get(),
        Phase::ShortBreak => SHORT_BREAK_MIN.get(),
        Phase::LongBreak => LONG_BREAK_MIN.get(),
        Phase::Idle => return 0,
    };
    min.saturating_mul(60).saturating_mul(1_000)
}

/// Progress fraction (1.0 = full, 0.0 = expired).
fn progress_fraction(phase: Phase) -> f32 {
    if phase == Phase::Idle {
        return 1.0;
    }
    let duration = phase_duration_ms(phase);
    if duration == 0 {
        return 1.0;
    }
    let elapsed = ELAPSED_MS.get();
    let remaining = duration.saturating_sub(elapsed);
    remaining as f32 / duration as f32
}

// ── Phase transitions ────────────────────────────────────────────────

fn transition_from_working() {
    let count = SESSION_COUNT.get() + 1;
    SESSION_COUNT.set(count);
    TOTAL_COMPLETED.set(TOTAL_COMPLETED.get() + 1);
    persist_total();

    if count >= SESSIONS_PER_CYCLE {
        // Cycle complete
        SESSION_COUNT.set(0);

        led_session_complete();
        play_chime(&CHIME_DONE);

        PHASE.set(Phase::LongBreak);
        ELAPSED_MS.set(0);
        RUNNING.set(true);
        apply_led_for_phase(Phase::LongBreak);
    } else {
        play_chime(&CHIME_BREAK);

        PHASE.set(Phase::ShortBreak);
        ELAPSED_MS.set(0);
        RUNNING.set(true);
        apply_led_for_phase(Phase::ShortBreak);
    }
}

fn transition_from_short_break() {
    play_chime(&CHIME_WORK);
    PHASE.set(Phase::Working);
    ELAPSED_MS.set(0);
    RUNNING.set(true);
    apply_led_for_phase(Phase::Working);
}

fn transition_from_long_break() {
    PHASE.set(Phase::Idle);
    ELAPSED_MS.set(0);
    RUNNING.set(false);
    apply_led_for_phase(Phase::Idle);
}

fn play_chime(audio: &Audio) {
    let id = ensure_audio_registered(audio);
    audio_play(id, Volume::FULL);
}

// ── Entry points ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    load_persisted_state();
}

/// Re-render in response to touch — the host no longer renders on touch by
/// itself, so an interactive widget must ask for the frame here.
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let size = widget_size();
    let phase = PHASE.get();
    let running = RUNNING.get();

    // Advance timer
    if running && phase != Phase::Idle {
        let elapsed = ELAPSED_MS.get() + delta_ms;
        let duration = phase_duration_ms(phase);

        if elapsed >= duration {
            // Phase expired
            match phase {
                Phase::Working => transition_from_working(),
                Phase::ShortBreak => transition_from_short_break(),
                Phase::LongBreak => transition_from_long_break(),
                Phase::Idle => {}
            }
        } else {
            ELAPSED_MS.set(elapsed);
        }
    }

    // Re-read after possible transitions
    let phase = PHASE.get();
    let running = RUNNING.get();

    let result = render_ui(
        size.width,
        size.height,
        build_timer_ui(size, phase, running),
    );

    handle_interactions(&result, phase, running);

    // Schedule next frame
    if running {
        request_frame_after(100); // 10 Hz
    } else {
        request_frame_after(1_000);
    }
}

// ── Interactions ─────────────────────────────────────────────────────

fn handle_interactions(result: &TreeRenderResult, phase: Phase, running: bool) {
    // Settings modal interactions (work even when modal is open)
    handle_settings_interactions(result);

    // Start/Pause
    if result.clicks.contains_key("start_pause") {
        if phase == Phase::Idle {
            // Start fresh work session
            PHASE.set(Phase::Working);
            ELAPSED_MS.set(0);
            RUNNING.set(true);
            apply_led_for_phase(Phase::Working);
            play_chime(&CHIME_WORK);
        } else if running {
            // Pause
            RUNNING.set(false);
            led::stop();
        } else {
            // Resume
            RUNNING.set(true);
            apply_led_for_phase(phase);
        }
    }

    // Stop
    if result.clicks.contains_key("stop") {
        PHASE.set(Phase::Idle);
        ELAPSED_MS.set(0);
        RUNNING.set(false);
        SESSION_COUNT.set(0);
        apply_led_for_phase(Phase::Idle);
    }

    // Open settings
    if result.clicks.contains_key("settings") {
        SETTINGS_OPEN.set(true);
    }
}

fn handle_settings_interactions(result: &TreeRenderResult) {
    let mut changed = false;

    let work_p = NumberInputProps {
        min: 1,
        max: 60,
        step: 1,
        ..Default::default()
    };
    let short_p = NumberInputProps {
        min: 1,
        max: 30,
        step: 1,
        ..Default::default()
    };
    let long_p = NumberInputProps {
        min: 1,
        max: 60,
        step: 1,
        ..Default::default()
    };

    if let Some(v) = number_input_handle("work", WORK_MIN.get() as i32, &work_p, result) {
        WORK_MIN.set(v as u32);
        changed = true;
    }
    if let Some(v) = number_input_handle("short", SHORT_BREAK_MIN.get() as i32, &short_p, result) {
        SHORT_BREAK_MIN.set(v as u32);
        changed = true;
    }
    if let Some(v) = number_input_handle("long", LONG_BREAK_MIN.get() as i32, &long_p, result) {
        LONG_BREAK_MIN.set(v as u32);
        changed = true;
    }

    if changed {
        save_u32("pomodoro_work_min", WORK_MIN.get());
        save_u32("pomodoro_short_break_min", SHORT_BREAK_MIN.get());
        save_u32("pomodoro_long_break_min", LONG_BREAK_MIN.get());
    }

    // Close/save
    if result.clicks.contains_key("settings::close")
        || result.clicks.contains_key("settings::primary")
    {
        SETTINGS_OPEN.set(false);
    }
}

// ── Timer UI ─────────────────────────────────────────────────────────

fn build_timer_ui(size: WidgetSize, phase: Phase, running: bool) -> Node {
    match size.variant {
        SizeVariant::Small => build_small(size, phase, running),
        SizeVariant::Medium => build_medium(size, phase, running),
        SizeVariant::Full | SizeVariant::Large => build_large(size, phase, running),
    }
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "Idle",
        Phase::Working => "Working",
        Phase::ShortBreak => "Short Break",
        Phase::LongBreak => "Long Break",
    }
}

fn phase_color(phase: Phase) -> Color {
    match phase {
        Phase::Idle => GRAY_50,
        Phase::Working => RED_50,
        Phase::ShortBreak => GREEN_50,
        Phase::LongBreak => BLUE_50,
    }
}

fn remaining_text(phase: Phase) -> String {
    if phase == Phase::Idle {
        let work = WORK_MIN.get();
        return pad2_colon(work, 0);
    }
    let duration = phase_duration_ms(phase);
    let elapsed = ELAPSED_MS.get();
    let remaining_ms = duration.saturating_sub(elapsed);
    let total_secs = remaining_ms / 1_000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    pad2_colon(mins, secs)
}

/// Format MM:SS with zero-padding (ufmt doesn't support {:02}).
fn pad2_colon(a: u32, b: u32) -> String {
    let m1 = a / 10;
    let m2 = a % 10;
    let s1 = b / 10;
    let s2 = b % 10;
    fmt!("{m1}{m2}:{s1}{s2}")
}

fn start_pause_button(_phase: Phase, running: bool) -> Node {
    if running {
        let icon_id = ensure_registered(&ICON_PAUSE);
        button!("start_pause", "", style: Secondary, size: Large, icon: icon_id)
    } else {
        let icon_id = ensure_registered(&ICON_PLAY);
        button!("start_pause", "", style: Primary, size: Large, icon: icon_id)
    }
}

fn stop_button() -> Node {
    let icon_id = ensure_registered(&ICON_STOP);
    button!("stop", "", style: Danger, size: Large, icon: icon_id)
}

fn settings_button() -> Node {
    let icon_id = ensure_registered(&ICON_SETTINGS);
    button!("settings", "", style: Ghost, size: Small, icon: icon_id)
}

/// Header row with optional title on the left and settings gear on the right.
fn header_row(title: Option<(&str, u32)>) -> Node {
    let mut children = Vec::new();
    if let Some((label, size)) = title {
        children.push(text(
            label.to_owned(),
            style!(size: size, weight: FontWeight::BOLD, color: GRAY_50),
        ));
    }
    children.push(spacer(1.0));
    children.push(settings_button());
    row(props!(), children)
}

/// 4 session dots with label — tracks progress through a 4-session pomodoro cycle.
fn session_dots(dot_size: f32, label_size: u32) -> Node {
    let count = SESSION_COUNT.get();
    let dots: Vec<Node> = (0..SESSIONS_PER_CYCLE)
        .map(|i| {
            let completed = i < count;
            let r = dot_size / 2.0;
            let color = if completed { GREEN_50 } else { GRAY_70 };
            let cell = dot_size + 4.0;
            let cx = cell / 2.0;
            let cy = cell / 2.0;
            canvas(
                props!(width: cell, height: cell),
                [Draw::circle(cx, cy, r, color)],
            )
        })
        .collect();
    row(
        props!(gap: 8.0),
        [
            text(
                fmt!("{}/{}", count, SESSIONS_PER_CYCLE),
                style!(size: label_size, color: GRAY_50),
            ),
            row(props!(gap: 6.0), dots),
        ],
    )
}

fn completed_text() -> Node {
    let total = TOTAL_COMPLETED.get();
    let label = if total == 1 {
        fmt!("{total} pomodoro today")
    } else {
        fmt!("{total} pomodoros today")
    };
    row(
        props!(inset_bottom: 8.0, inset_left: 8.0),
        [text(label, style!(size: 14, color: GRAY_50))],
    )
}

/// Progress bar showing remaining time.
///
/// Two layers: dark base track (GRAY_90) + centered value bar that shrinks
/// from both sides as time depletes. Color transitions from bright neutral
/// (GRAY_30) toward red as time runs out.
fn time_progress(phase: Phase, track_h: f32) -> Node {
    let frac = progress_fraction(phase);
    let value_color = progress_color(frac);

    // Center-shrinking: equal empty space on both sides
    let fill = (frac * 1_000.0) as u32;
    let gap = 1_000 - fill;
    let gap_left = gap / 2;
    let gap_right = gap - gap_left;

    let mut value_children: Vec<Node> = Vec::with_capacity(3);
    if gap_left > 0 {
        value_children.push(row(props!(flex: gap_left as f32, height: track_h), []));
    }
    if fill > 0 {
        value_children.push(row(
            props!(flex: fill as f32, background: value_color, height: track_h),
            [],
        ));
    }
    if gap_right > 0 {
        value_children.push(row(props!(flex: gap_right as f32, height: track_h), []));
    }

    // Base: always-visible dark track, one step lighter than widget bg
    // Value: overlaid via absolute positioning
    col(
        props!(
            background: GRAY_90,
            height: track_h,
            inset_bottom: 0.0,
            inset_left: 0.0,
            inset_right: 0.0
        ),
        [row(
            props!(
                inset_top: 0.0,
                inset_bottom: 0.0,
                inset_left: 0.0,
                inset_right: 0.0
            ),
            value_children,
        )],
    )
}

/// Value bar color: bright neutral at 100% → red at 0%.
fn progress_color(frac: f32) -> Color {
    // Full = GRAY_30 (bright silver), depleted = RED_50 (urgent)
    let t = 1.0 - frac; // 0.0 at full, 1.0 at empty
    GRAY_30.mix(RED_50, t)
}

/// Settings modal overlay.
fn settings_modal(size: WidgetSize) -> Node {
    let work = WORK_MIN.get();
    let short = SHORT_BREAK_MIN.get();
    let long = LONG_BREAK_MIN.get();

    let is_half = matches!(size.variant, SizeVariant::Small | SizeVariant::Medium);
    let margin: u16 = if is_half { 12 } else { 48 };
    // 3 number inputs (~53px each) + 2 gaps (8px) + helper/error text (~32px)
    let content_h = 250.0;

    modal(
        "settings",
        SETTINGS_OPEN.get(),
        "Settings",
        vec![
            number_input!("work", work as i32, label: "Work", suffix: "min", min: 1, max: 60),
            number_input!("short", short as i32, label: "Short break", suffix: "min", min: 1, max: 30),
            number_input!("long", long as i32, label: "Long break", suffix: "min", min: 1, max: 60),
        ],
        Some(ModalProps {
            height: content_h,
            margin,
            max_width: SizeVariant::Medium.width() as u16,
            footer: Some(ModalFooter {
                primary: ModalAction { label: "Save" },
                secondary: None,
                danger: false,
            }),
            ..ModalProps::default()
        }),
    )
}

// ── Small layout (317×238) ───────────────────────────────────────────

fn build_small(size: WidgetSize, phase: Phase, running: bool) -> Node {
    col(
        props!(background: GRAY_100, padding: 8.0, gap: 4.0),
        [
            header_row(Some(("Pomodoro", 14))),
            spacer(2.0),
            row(
                props!(gap: 8.0),
                [
                    col(
                        props!(flex: 1.0, gap: 4.0),
                        [
                            center(
                                props!(),
                                [text(
                                    phase_label(phase),
                                    style!(size: 14, weight: FontWeight::BOLD, color: phase_color(phase)),
                                )],
                            ),
                            center(
                                props!(),
                                [text(
                                    remaining_text(phase),
                                    style!(size: 48, weight: FontWeight::BOLD, color: WHITE),
                                )],
                            ),
                            center(props!(), [session_dots(10.0, 12)]),
                        ],
                    ),
                    col(
                        props!(gap: 4.0),
                        [
                            spacer(1.0),
                            start_pause_button(phase, running),
                            stop_button(),
                            spacer(1.0),
                        ],
                    ),
                ],
            ),
            spacer(6.0),
            completed_text(),
            time_progress(phase, 3.0),
            settings_modal(size),
        ],
    )
}

// ── Medium layout (638×238) ──────────────────────────────────────────

fn build_medium(size: WidgetSize, phase: Phase, running: bool) -> Node {
    col(
        props!(background: GRAY_100, padding: 12.0, gap: 4.0),
        [
            header_row(Some(("Pomodoro", 18))),
            row(
                props!(gap: 12.0),
                [
                    col(
                        props!(flex: 1.0, gap: 4.0),
                        [
                            center(
                                props!(),
                                [text(
                                    phase_label(phase),
                                    style!(size: 16, weight: FontWeight::BOLD, color: phase_color(phase)),
                                )],
                            ),
                            center(
                                props!(),
                                [text(
                                    remaining_text(phase),
                                    style!(size: 64, weight: FontWeight::BOLD, color: WHITE),
                                )],
                            ),
                            center(props!(), [session_dots(14.0, 14)]),
                        ],
                    ),
                    col(
                        props!(gap: 8.0),
                        [
                            spacer(1.0),
                            start_pause_button(phase, running),
                            stop_button(),
                            spacer(1.0),
                        ],
                    ),
                ],
            ),
            completed_text(),
            time_progress(phase, 3.0),
            settings_modal(size),
        ],
    )
}

// ── Large/Full layout ────────────────────────────────────────────────

fn build_large(size: WidgetSize, phase: Phase, running: bool) -> Node {
    let is_full = size.variant == SizeVariant::Full;
    let pad = if is_full { 20.0 } else { 16.0 };
    let timer_size: u32 = if is_full { 120 } else { 96 };
    let title_size: u32 = if is_full { 24 } else { 20 };
    let dot_size = if is_full { 20.0 } else { 16.0 };
    let track_h = if is_full { 4.0 } else { 3.0 };

    col(
        props!(background: GRAY_100, padding: pad, gap: 8.0),
        [
            header_row(Some(("Pomodoro", title_size))),
            spacer(1.0),
            center(
                props!(),
                [text(
                    phase_label(phase),
                    style!(size: 20, weight: FontWeight::BOLD, color: phase_color(phase)),
                )],
            ),
            row(
                props!(gap: 16.0),
                [
                    spacer(1.0),
                    col(
                        props!(gap: 4.0),
                        [
                            center(
                                props!(),
                                [text(
                                    remaining_text(phase),
                                    style!(size: timer_size, weight: FontWeight::BOLD, color: WHITE, line_height: 1.0),
                                )],
                            ),
                            center(props!(), [session_dots(dot_size, 16)]),
                        ],
                    ),
                    col(
                        props!(gap: 8.0),
                        [
                            spacer(1.0),
                            start_pause_button(phase, running),
                            stop_button(),
                            spacer(1.0),
                        ],
                    ),
                    spacer(1.0),
                ],
            ),
            spacer(2.0),
            time_progress(phase, track_h),
            completed_text(),
            settings_modal(size),
        ],
    )
}
