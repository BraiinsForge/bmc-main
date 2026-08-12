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

#![allow(clippy::cast_precision_loss)]

//! Home Assistant Widget — WebSocket SDK POC (BDK-266).
//!
//! Connects to a Home Assistant instance via WebSocket, authenticates,
//! subscribes to `state_changed` events, and renders a live entity list.

use std::cell::{Cell, RefCell};

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;
// ── Configuration ─────────────────────────────────────────────────
// Configure via KV store: ha_url, ha_token
const DEFAULT_HA_URL: &str = "ws://homeassistant.lan:8123/api/websocket";

// ── State ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct EntityState {
    entity_id: String,
    friendly_name: Option<String>,
    state: String,
    unit: Option<String>,
    /// Domain extracted from entity_id (e.g. "light", "sensor", "switch").
    domain: String,
}

enum HaState {
    Connecting,
    Authenticating,
    Subscribing,
    Live(Vec<EntityState>),
    Error(String),
}

thread_local! {
    static STATE: RefCell<HaState> = const { RefCell::new(HaState::Connecting) };
    /// Monotonic message ID for the HA JSON-RPC protocol.
    static MSG_ID: Cell<u32> = const { Cell::new(1) };
    /// Message ID used for the `get_states` request (to match the result).
    static STATES_MSG_ID: Cell<u32> = const { Cell::new(0) };
}

/// Format a float with one decimal place (ufmt doesn't support precision specifiers).
fn fmt_f1(v: f32) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let scaled = (v * 10.0).round() as i32;
    let whole = scaled / 10;
    let frac = (scaled % 10).unsigned_abs();
    fmt!("{}.{}", whole, frac)
}

/// Format an ISO 8601 date string compactly using the host's chrono formatter.
fn fmt_iso_date(s: &str) -> Option<String> {
    let ts = parse_datetime(s)?;
    Some(strftime(ts, "%d/%m/%Y %H:%M:%S"))
}

fn domain_of(entity_id: &str) -> String {
    entity_id
        .split_once('.')
        .map_or_else(|| entity_id.to_string(), |(d, _)| d.to_string())
}

fn next_msg_id() -> u32 {
    MSG_ID.with(|id| {
        let v = id.get();
        id.set(v + 1);
        v
    })
}

// ── WASM exports ──────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let url = kv::get_string("ha_url").unwrap_or_else(|| DEFAULT_HA_URL.into());
    if ws!(&url, on_ha_event).is_none() {
        let msg = "HA WebSocket rejected by host runtime limits".to_string();
        log_error!("{}", msg);
        STATE.with(|s| *s.borrow_mut() = HaState::Error(msg));
        request_frame();
    }
}

/// Re-render in response to touch — the host no longer renders on touch by
/// itself, so an interactive widget must ask for the frame here.
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = widget_size();

    let is_small = matches!(size.variant, SizeVariant::Small | SizeVariant::Medium);
    let pad = if is_small { 12.0 } else { 24.0 };
    let gap = if is_small { 8.0 } else { 16.0 };

    let root = STATE.with(|s| {
        let borrow = s.borrow();
        match &*borrow {
            HaState::Connecting => status_screen(size, "Connecting\u{2026}"),
            HaState::Authenticating => status_screen(size, "Authenticating\u{2026}"),
            HaState::Subscribing => status_screen(size, "Subscribing\u{2026}"),
            HaState::Error(msg) => col(
                props!(padding: pad, gap: gap, background: BLACK),
                [
                    header(is_small),
                    notification(NotificationKind::Error, "Connection error", msg),
                ],
            ),
            HaState::Live(entities) => render_entities(size, entities),
        }
    });

    let _ = render_ui(size.width, size.height, root);
}

// ── WebSocket event handler ───────────────────────────────────────

fn on_ha_event(ws: Ws, event: &WsEvent) {
    match event {
        WsEvent::Open => {
            log_info!("HA WebSocket connected, waiting for auth_required");
            STATE.with(|s| *s.borrow_mut() = HaState::Authenticating);
            request_frame();
        }
        WsEvent::Message(text) => handle_message(ws, text),
        WsEvent::Close(code) => {
            let msg = fmt!("WebSocket closed (code {})", code);
            log_warn!("{}", msg);
            STATE.with(|s| *s.borrow_mut() = HaState::Error(msg));
            request_frame();
        }
    }
}

fn handle_message(ws: Ws, text: &str) {
    let json = JsonDoc::parse(text.as_bytes());
    let msg_type = json.str("/type").unwrap_or_default();

    match msg_type.as_str() {
        "auth_required" => {
            log_info!("HA requires auth, sending token");
            let id = next_msg_id();
            let token = kv::get_string("ha_token").unwrap_or_default();
            let auth_msg = fmt!(r#"{{"type":"auth","access_token":"{}"}}"#, token);
            // HA auth messages don't use the id field, but we burn one to keep
            // the counter monotonic for later subscribe calls.
            let _ = id;
            ws.send(&auth_msg);
        }
        "auth_ok" => {
            log_info!("HA auth OK, fetching states + subscribing");
            STATE.with(|s| *s.borrow_mut() = HaState::Subscribing);

            // Fetch all current entity states
            let states_id = next_msg_id();
            STATES_MSG_ID.set(states_id);
            ws.send(&fmt!(r#"{{"id":{},"type":"get_states"}}"#, states_id));

            // Subscribe to live updates
            let sub_id = next_msg_id();
            ws.send(&fmt!(
                r#"{{"id":{},"type":"subscribe_events","event_type":"state_changed"}}"#,
                sub_id
            ));
            request_frame();
        }
        "auth_invalid" => {
            let reason = json
                .str("/message")
                .unwrap_or_else(|| "invalid token".into());
            log_error!("HA auth failed: {}", reason);
            STATE.with(|s| *s.borrow_mut() = HaState::Error(reason));
            ws.close();
            request_frame();
        }
        "result" => handle_result(&json),
        "event" => handle_state_changed(&json),
        _ => {}
    }
}

fn handle_result(json: &JsonDoc) {
    let success = json.bool("/success").unwrap_or(false);
    if !success {
        let err = json
            .str("/error/message")
            .unwrap_or_else(|| "unknown error".into());
        log_error!("HA request failed: {}", err);
        STATE.with(|s| *s.borrow_mut() = HaState::Error(err));
        request_frame();
        return;
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let msg_id = json.i64("/id").unwrap_or(0) as u32;
    let states_id = STATES_MSG_ID.get();

    if msg_id == states_id && states_id != 0 {
        // Response to get_states — bulk-load all entities
        let mut entities = Vec::new();
        for i in 0.. {
            let path_id = fmt!("/result/{}/entity_id", i);
            let path_state = fmt!("/result/{}/state", i);
            let path_name = fmt!("/result/{}/attributes/friendly_name", i);
            let path_unit = fmt!("/result/{}/attributes/unit_of_measurement", i);
            let Some(entity_id) = json.str(&path_id) else {
                break;
            };
            let state = json.str(&path_state).unwrap_or_else(|| "unknown".into());
            if state == "unavailable" || state == "unknown" {
                continue;
            }
            let domain = domain_of(&entity_id);
            let friendly_name = json.str(&path_name);
            let unit = json.str(&path_unit);
            entities.push(EntityState {
                entity_id,
                friendly_name,
                state,
                unit,
                domain,
            });
        }
        entities.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        log_info!("loaded {} entities", entities.len());
        STATE.with(|s| *s.borrow_mut() = HaState::Live(entities));
    } else {
        // Subscription confirmed or other result
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            if matches!(*st, HaState::Subscribing) {
                *st = HaState::Live(Vec::new());
            }
        });
    }
    request_frame();
}

fn handle_state_changed(json: &JsonDoc) {
    let entity_id = json
        .str("/event/data/new_state/entity_id")
        .unwrap_or_default();
    let state_val = json
        .str("/event/data/new_state/state")
        .unwrap_or_else(|| "unknown".into());

    if entity_id.is_empty() {
        return;
    }

    let friendly_name = json.str("/event/data/new_state/attributes/friendly_name");
    let unit = json.str("/event/data/new_state/attributes/unit_of_measurement");
    let is_unavailable = state_val == "unavailable" || state_val == "unknown";
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if let HaState::Live(entities) = &mut *st {
            if is_unavailable {
                entities.retain(|e| e.entity_id != entity_id);
            } else if let Some(existing) = entities.iter_mut().find(|e| e.entity_id == entity_id) {
                existing.state = state_val;
                if friendly_name.is_some() {
                    existing.friendly_name = friendly_name;
                }
                if unit.is_some() {
                    existing.unit = unit;
                }
            } else {
                let domain = domain_of(&entity_id);
                entities.push(EntityState {
                    entity_id,
                    friendly_name,
                    state: state_val,
                    unit,
                    domain,
                });
                entities.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
            }
        }
    });
    request_frame();
}

// ── Rendering ─────────────────────────────────────────────────────

fn header(is_small: bool) -> Node {
    let title_size = if is_small { 16 } else { 24 };
    col(
        props!(),
        [
            row(
                props!(gap: 8.0),
                [
                    text(
                        "Home Assistant",
                        style!(size: title_size, weight: FontWeight::BOLD),
                    ),
                    text("\u{2022}", style!(size: title_size, color: GRAY_50)),
                    status_indicator(is_small),
                ],
            ),
            text(
                DEFAULT_HA_URL,
                style!(size: if is_small { 10 } else { 12 }, color: GRAY_40),
            ),
        ],
    )
}

fn status_indicator(is_small: bool) -> Node {
    STATE.with(|s| {
        let borrow = s.borrow();
        let (label, color) = match &*borrow {
            HaState::Connecting | HaState::Authenticating | HaState::Subscribing => {
                ("connecting", YELLOW_50)
            }
            HaState::Live(_) => ("live", GREEN_50),
            HaState::Error(_) => ("error", RED_50),
        };
        text(
            label,
            style!(size: if is_small { 16 } else { 24 }, color: color),
        )
    })
}

fn status_screen(size: WidgetSize, msg: &str) -> Node {
    let is_small = matches!(size.variant, SizeVariant::Small | SizeVariant::Medium);
    let pad = if is_small { 12.0 } else { 24.0 };
    let msg_size = if is_small { 14 } else { 20 };
    col(
        props!(padding: pad, gap: 8.0, background: BLACK),
        [
            header(is_small),
            text(msg, style!(size: msg_size, color: GRAY_30)),
        ],
    )
}

fn render_entities(size: WidgetSize, entities: &[EntityState]) -> Node {
    let is_small = matches!(size.variant, SizeVariant::Small | SizeVariant::Medium);
    let pad = if is_small { 12.0 } else { 24.0 };
    let gap = if is_small { 8.0 } else { 16.0 };
    // Header height: title (~24/16px) + url (~12/10px) + gap(2) ≈ 38/28px
    let header_h = if is_small { 28.0 } else { 38.0 };

    if entities.is_empty() {
        let msg_size = if is_small { 14 } else { 20 };
        return col(
            props!(padding: pad, gap: gap, background: BLACK),
            [
                header(is_small),
                text(
                    "Waiting for state changes\u{2026}",
                    style!(size: msg_size, color: GRAY_30),
                ),
            ],
        );
    }

    let scroll_height = (size.height as f32) - pad - header_h - gap - pad;
    // Cap to avoid layout perf hit — taffy measures every node even if clipped
    let max_entities = 50;
    let entity_rows: Vec<Node> = entities
        .iter()
        .take(max_entities)
        .map(|e| entity_row(e, size.variant))
        .collect();

    col(
        props!(padding: pad, gap: gap, background: BLACK),
        [
            header(is_small),
            scroll(
                "entities",
                props!(height: scroll_height, gap: 4.0, max_width: 650.0),
                entity_rows,
            ),
        ],
    )
}

/// Fixed width for the right-hand value/gauge column.
const VALUE_COL_W: f32 = 140.0;

fn entity_row(entity: &EntityState, variant: SizeVariant) -> Node {
    let font_size = match variant {
        SizeVariant::Small | SizeVariant::Medium => 12,
        SizeVariant::Large | SizeVariant::Full => 14,
    };

    let label = entity.friendly_name.as_deref().unwrap_or(&entity.entity_id);

    let numeric: Option<f32> = entity.state.parse().ok();

    // Gauge bar with overlaid value for numeric entities
    if let Some(value) = numeric {
        let unit = entity.unit.as_deref().unwrap_or("");
        let (lo, hi, color) = gauge_range(unit, &entity.domain);
        let frac = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
        let v = fmt_f1(value);
        let value_text = if unit.is_empty() {
            v
        } else {
            fmt!("{} {}", v, unit)
        };

        return row(
            props!(gap: 8.0),
            [
                text(label, style!(size: font_size, color: GRAY_30)),
                spacer(1.0),
                gauge_bar_with_label(&value_text, frac, color, font_size),
            ],
        );
    }

    // On/off, dates, and other non-numeric entities — right-aligned in same-width column
    let display_value = fmt_iso_date(&entity.state).unwrap_or_else(|| entity.state.clone());
    let state_color = match entity.state.as_str() {
        "on" => GREEN_50,
        "off" => RED_50,
        _ => GRAY_30,
    };

    row(
        props!(gap: 8.0),
        [
            text(label, style!(size: font_size, color: GRAY_30)),
            spacer(1.0),
            row(
                props!(width: VALUE_COL_W),
                [
                    spacer(1.0),
                    text(
                        &display_value,
                        style!(size: font_size, weight: FontWeight::BOLD, color: state_color, text_overflow: TextOverflow::Clip),
                    ),
                ],
            ),
        ],
    )
}

/// Determine gauge range (min, max) and color from unit or domain.
fn gauge_range(unit: &str, domain: &str) -> (f32, f32, Color) {
    match unit {
        "%" => (0.0, 100.0, BLUE_50),
        "\u{b0}C" | "\u{b0}F" => {
            let hi = if unit == "\u{b0}F" { 120.0 } else { 50.0 };
            (0.0, hi, YELLOW_50)
        }
        "W" | "kW" => (0.0, if unit == "kW" { 10.0 } else { 3_000.0 }, ORANGE_50),
        "lx" => (0.0, 1_000.0, YELLOW_30),
        "hPa" | "mbar" => (950.0, 1_050.0, TEAL_50),
        _ => match domain {
            "sensor" => (0.0, 100.0, BLUE_30),
            _ => (0.0, 100.0, GRAY_30),
        },
    }
}

/// Gauge bar with value text overlaid. Text gets a dark outline for readability.
fn gauge_bar_with_label(label: &str, frac: f32, bg_color: Color, font_size: u32) -> Node {
    let ow = 3.0; // outline padding
    let w = VALUE_COL_W;
    let bar_h = font_size as f32;
    let h = bar_h + ow * 2.0;
    let fill_w = frac * w;
    let tx = w / 2.0;
    let ty = ow - 2.0;

    canvas(
        props!(width: w, height: h),
        vec![
            Draw::rect(0.0, ow, w, bar_h, GRAY_60),
            Draw::rect(0.0, ow, fill_w, bar_h, bg_color),
            Draw::text(
                tx,
                ty,
                label,
                style!(
                    size: font_size,
                    weight: FontWeight::BOLD,
                    color: WHITE,
                    align: TextAlign::Center,
                    outline_color: bg_color.brightness(0.4),
                    outline_width: 2.0,
                ),
            ),
        ],
    )
}
