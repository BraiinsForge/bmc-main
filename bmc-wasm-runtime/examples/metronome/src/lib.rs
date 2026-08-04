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
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

//! Metronome widget — BPM control with audible click track.

use std::cell::Cell;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

const TICK: Audio = include_audio!("assets/sounds/Perc_MetronomeQuartz_lo.wav");
const ACCENT: Audio = include_audio!("assets/sounds/Perc_MetronomeQuartz_hi.wav");

const MIN_BPM: u32 = 30;
const MAX_BPM: u32 = 300;
const BEATS_PER_BAR: u32 = 4;
const TAP_HISTORY: usize = 4;

thread_local! {
    static BPM: Cell<u32> = const { Cell::new(120) };
    static PLAYING: Cell<bool> = const { Cell::new(false) };
    static BEAT: Cell<u32> = const { Cell::new(0) };
    static ELAPSED_MS: Cell<u64> = const { Cell::new(0) };
    /// Monotonic clock that always advances; used for tap tempo timing.
    static WALL_MS: Cell<u64> = const { Cell::new(0) };
    static TAP_COUNT: Cell<usize> = const { Cell::new(0) };
    static TAP_TIMES: Cell<[u64; TAP_HISTORY]> = const { Cell::new([0; TAP_HISTORY]) };
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
    let bpm = BPM.get();
    let playing = PLAYING.get();
    let beat = BEAT.get();

    // Always advance wall clock (for tap tempo)
    WALL_MS.set(WALL_MS.get() + u64::from(delta_ms));

    // Advance beat timing
    if playing {
        let mut elapsed = ELAPSED_MS.get() + u64::from(delta_ms);
        let interval_ms = 60_000 / u64::from(bpm);
        let mut current_beat = beat;

        while elapsed >= interval_ms {
            elapsed -= interval_ms;
            current_beat = (current_beat + 1) % BEATS_PER_BAR;
            BEAT.set(current_beat);

            // Play accent on beat 0, tick on others
            let tick_id = ensure_audio_registered(&TICK);
            let accent_id = ensure_audio_registered(&ACCENT);
            if current_beat == 0 {
                audio_play(accent_id, Volume::FULL);
            } else {
                audio_play(tick_id, Volume::new(80));
            }
        }
        ELAPSED_MS.set(elapsed);
    }

    let result = render_ui(size.width, size.height, build_ui(size, bpm, playing, beat));

    // Handle interactions
    if result.clicks.contains_key("play_stop") {
        if playing {
            PLAYING.set(false);
            // Hard-cut the in-flight click so Stop is audibly immediate
            // instead of trailing the last sample to its natural end.
            audio_stop(ensure_audio_registered(&TICK));
            audio_stop(ensure_audio_registered(&ACCENT));
        } else {
            PLAYING.set(true);
            BEAT.set(0);
            ELAPSED_MS.set(0);

            // Play accent immediately on start
            let accent_id = ensure_audio_registered(&ACCENT);
            audio_play(accent_id, Volume::FULL);
        }
    }

    if result.clicks.contains_key("bpm_minus") {
        BPM.set(bpm.saturating_sub(1).max(MIN_BPM));
    }
    if result.clicks.contains_key("bpm_plus") {
        BPM.set((bpm + 1).min(MAX_BPM));
    }
    if result.clicks.contains_key("bpm_minus_10") {
        BPM.set(bpm.saturating_sub(10).max(MIN_BPM));
    }
    if result.clicks.contains_key("bpm_plus_10") {
        BPM.set((bpm + 10).min(MAX_BPM));
    }

    // Tap tempo
    if result.clicks.contains_key("tap_tempo") {
        handle_tap_tempo();
    }

    // BPM slider drag
    if let Some(hit) = host::get_touch_drag("bpm_slider") {
        let frac = hit.frac_x();
        let new_bpm = MIN_BPM + ((MAX_BPM - MIN_BPM) as f32 * frac) as u32;
        BPM.set(new_bpm.clamp(MIN_BPM, MAX_BPM));
    }

    // Schedule next frame
    if PLAYING.get() {
        request_frame_after(16);
    } else {
        request_frame_after(100);
    }
}

fn handle_tap_tempo() {
    let now = WALL_MS.get();
    let count = TAP_COUNT.get();

    let mut times = TAP_TIMES.get();
    let idx = count % TAP_HISTORY;
    times[idx] = now;
    TAP_TIMES.set(times);
    TAP_COUNT.set(count + 1);

    let used = (count + 1).min(TAP_HISTORY);
    if used >= 2 {
        // Calculate average interval between taps
        let mut intervals = 0u64;
        let mut interval_count = 0u32;
        for i in 1..used {
            let prev_idx = (count + 1 - used + i - 1) % TAP_HISTORY;
            let curr_idx = (count + 1 - used + i) % TAP_HISTORY;
            let dt = times[curr_idx].saturating_sub(times[prev_idx]);
            if dt > 0 && dt < 3_000 {
                intervals += dt;
                interval_count += 1;
            }
        }
        if interval_count > 0 {
            let avg_ms = intervals / u64::from(interval_count);
            if avg_ms > 0 {
                let new_bpm = (60_000 / avg_ms) as u32;
                BPM.set(new_bpm.clamp(MIN_BPM, MAX_BPM));
            }
        }
    }
}

// ── Layout ──────────────────────────────────────────────────────────

fn build_ui(size: WidgetSize, bpm: u32, playing: bool, beat: u32) -> Node {
    match size.variant {
        SizeVariant::Small => build_small(bpm, playing, beat),
        SizeVariant::Medium => build_medium(bpm, playing, beat),
        SizeVariant::Large | SizeVariant::Full => build_large(size, bpm, playing, beat),
    }
}

/// Small (317×238) — compact vertical stack, no title, smaller fonts.
fn build_small(bpm: u32, playing: bool, beat: u32) -> Node {
    col(
        props!(background: GRAY_100, padding: 12.0, gap: 8.0),
        [
            // BPM display
            center(
                props!(),
                [text(
                    fmt!("{} BPM", bpm),
                    style!(size: 36, weight: FontWeight::BOLD, color: WHITE),
                )],
            ),
            // Tap tempo (centered via spacers)
            row(
                props!(),
                [
                    spacer(1.0),
                    button!("tap_tempo", "Tap Tempo", style: Tertiary, size: Small),
                    spacer(1.0),
                ],
            ),
            // Beat dots
            center(props!(), [beat_indicators(beat, playing, 16.0)]),
            // Slider (smaller thumb on small layout)
            bpm_slider(bpm, 2.0),
            // Transport row:  -10  -1  [Play]  +1  +10
            transport_row(playing),
        ],
    )
}

/// Medium (638×238) — two-column layout: BPM display left, controls right.
fn build_medium(bpm: u32, playing: bool, beat: u32) -> Node {
    row(
        props!(background: GRAY_100, padding: 16.0, gap: 20.0),
        [
            // Left column: BPM display + beat dots + tap tempo
            col(
                props!(flex: 1.0, gap: 8.0),
                [
                    spacer(1.0),
                    center(
                        props!(),
                        [text(
                            fmt!("{} BPM", bpm),
                            style!(size: 64, weight: FontWeight::BOLD, color: WHITE),
                        )],
                    ),
                    row(
                        props!(),
                        [
                            spacer(1.0),
                            button!("tap_tempo", "Tap Tempo", style: Tertiary, size: Small),
                            spacer(1.0),
                        ],
                    ),
                    center(props!(), [beat_indicators(beat, playing, 20.0)]),
                    spacer(1.0),
                ],
            ),
            // Right column: slider, transport
            col(
                props!(flex: 1.0, gap: 10.0),
                [
                    spacer(1.0),
                    bpm_slider(bpm, 4.0),
                    transport_row(playing),
                    spacer(1.0),
                ],
            ),
        ],
    )
}

/// Large/Full — spacious single-column layout centered vertically.
fn build_large(size: WidgetSize, bpm: u32, playing: bool, beat: u32) -> Node {
    let is_full = size.variant == SizeVariant::Full;
    let title_size: u32 = if is_full { 24 } else { 20 };
    let bpm_size: u32 = if is_full { 80 } else { 64 };
    let dot_size = if is_full { 22.0 } else { 18.0 };
    let pad = if is_full { 24.0 } else { 16.0 };
    let gap = if is_full { 12.0 } else { 8.0 };

    col(
        props!(background: GRAY_100, padding: pad, gap: gap),
        [
            // Title
            text(
                "Metronome",
                style!(size: title_size, weight: FontWeight::BOLD, color: GRAY_50),
            ),
            spacer(1.0),
            // BPM display
            center(
                props!(),
                [text(
                    fmt!("{} BPM", bpm),
                    style!(size: bpm_size, weight: FontWeight::BOLD, color: WHITE),
                )],
            ),
            // Tap tempo (centered via spacers)
            row(
                props!(),
                [
                    spacer(1.0),
                    button!("tap_tempo", "Tap Tempo", style: Tertiary, size: Small),
                    spacer(1.0),
                ],
            ),
            // Beat indicator dots
            center(props!(), [beat_indicators(beat, playing, dot_size)]),
            spacer(1.0),
            // Slider
            bpm_slider(bpm, if is_full { 6.0 } else { 4.0 }),
            // Transport:  -10  -1  [Play/Stop]  +1  +10
            transport_row(playing),
        ],
    )
}

// ── Components ──────────────────────────────────────────────────────

/// Beat indicator dots with smooth size/color transition.
///
/// Each dot uses `.transition()` so the size and color animate smoothly
/// between the active/inactive states instead of snapping.
fn beat_indicators(beat: u32, playing: bool, max_r: f32) -> Node {
    let dots: Vec<Node> = (0..BEATS_PER_BAR)
        .map(|i| {
            let is_accent = i == 0;
            let is_active = playing && i == beat;

            // Active dots are full-size, inactive ones are small
            let r = if is_active { max_r / 2.0 } else { max_r / 3.0 };
            let color = if is_active {
                if is_accent { RED_50 } else { GREEN_50 }
            } else if is_accent {
                GRAY_50
            } else {
                GRAY_70
            };

            let cell = max_r + 4.0;
            let cx = cell / 2.0;
            let cy = cell / 2.0;
            canvas(
                props!(width: cell, height: cell),
                [Draw::circle(cx, cy, r, color).transition("dot", 150, Easing::EaseOut)],
            )
        })
        .collect();

    row(props!(gap: 8.0), dots)
}

/// BPM slider — built-in progress_bar with touch_key for drag interaction.
/// Playhead dot radius = track_h * 2, so keep track_h small on tight layouts.
fn bpm_slider(bpm: u32, track_h: f32) -> Node {
    let frac = (bpm - MIN_BPM) as f32 / (MAX_BPM - MIN_BPM) as f32;
    progress_bar!(ProgressMode::Slider(frac),
        touch_key: "bpm_slider", track_h: track_h,
        fill_color: GREEN_50, track_color: GRAY_80, bg_color: TRANSPARENT,
    )
}

/// Transport row:  -10  -1  [Play/Stop]  +1  +10
fn transport_row(playing: bool) -> Node {
    row(
        props!(gap: 8.0),
        [
            button!("bpm_minus_10", "-10", style: Secondary, size: Small),
            button!("bpm_minus", "-1", style: Secondary, size: Small),
            spacer(1.0),
            if playing {
                button!("play_stop", "Stop", style: Danger, size: Large)
            } else {
                button!("play_stop", "Play", style: Primary, size: Large)
            },
            spacer(1.0),
            button!("bpm_plus", "+1", style: Secondary, size: Small),
            button!("bpm_plus_10", "+10", style: Secondary, size: Small),
        ],
    )
}
