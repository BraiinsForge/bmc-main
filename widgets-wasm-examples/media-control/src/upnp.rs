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

//! UPnP/DLNA media controller — SOAP over HTTP POST.
//!
//! All protocol logic lives here. Uses `FetchRequest::post()` for SOAP actions
//! and `XmlDoc` for parsing responses. No crates beyond the SDK.
//!
//! # Protocol summary
//!
//! UPnP AVTransport (playback) and RenderingControl (volume) services are
//! controlled via SOAP-over-HTTP POST. Each action is a POST to the device's
//! control URL with a SOAP XML body and a `SOAPAction` header.
//!
//! Responses are XML with a SOAP envelope. Track metadata is embedded as
//! DIDL-Lite XML inside a `<TrackMetaData>` element (double-encoded).

use bmc_wasm_sdk::{FetchRequest, FetchResponse, XmlDoc, fmt, log_warn, ufmt};

// ── Service URNs ─────────────────────────────────────────────────

const AV_TRANSPORT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
const RENDERING_CONTROL: &str = "urn:schemas-upnp-org:service:RenderingControl:1";

// ── Connection info ──────────────────────────────────────────────

/// Discovered UPnP device endpoints.
///
/// For POC: hardcoded from the device's description XML.
/// Future: populated by SSDP discovery (Stage 5).
pub struct UpnpDevice {
    /// Base URL of the device (e.g. `http://192.168.1.50:49152`).
    pub base_url: String,
    /// Control URL path for AVTransport (e.g. `/AVTransport/Control`).
    pub av_transport_path: String,
    /// Control URL path for RenderingControl (e.g. `/RenderingControl/Control`).
    pub rendering_control_path: String,
}

impl UpnpDevice {
    fn av_transport_url(&self) -> String {
        fmt!("{}{}", self.base_url, self.av_transport_path)
    }

    fn rendering_control_url(&self) -> String {
        fmt!("{}{}", self.base_url, self.rendering_control_path)
    }
}

// ── SOAP envelope builder ────────────────────────────────────────

/// Build a SOAP envelope for a UPnP action.
///
/// `service_urn` is the full service URN (e.g. `AV_TRANSPORT`).
/// `action` is the action name (e.g. `Play`, `GetPositionInfo`).
/// `args` is a list of `(name, value)` pairs for the action body.
fn soap_envelope(service_urn: &str, action: &str, args: &[(&str, &str)]) -> String {
    let mut body = String::with_capacity(512);
    body.push_str(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body>"#,
    );
    body.push_str(&fmt!("<u:{} xmlns:u=\"{}\">", action, service_urn));
    for (name, value) in args {
        body.push_str(&fmt!("<{}>{}</{}>", name, value, name));
    }
    body.push_str(&fmt!("</u:{}>", action));
    body.push_str("</s:Body></s:Envelope>");
    body
}

/// Build the `SOAPAction` header value for a UPnP action.
fn soap_action_header(service_urn: &str, action: &str) -> String {
    fmt!("\"{}#{}\"", service_urn, action)
}

// ── SOAP request dispatcher ──────────────────────────────────────

/// Send a SOAP action, optionally after a delay.
fn soap_request_impl(
    delay_ms: Option<u32>,
    url: &str,
    service_urn: &str,
    action: &str,
    args: &[(&str, &str)],
    cb: fn(&FetchResponse),
) {
    let envelope = soap_envelope(service_urn, action, args);
    let soap_action = soap_action_header(service_urn, action);
    let headers = fmt!(
        "Content-Type: text/xml; charset=\"utf-8\"\nSOAPAction: {}",
        soap_action,
    );

    let req = FetchRequest::post(url)
        .headers(&headers)
        .body(envelope.as_bytes());
    if let Some(ms) = delay_ms {
        if req.send_after(ms, cb).is_none() {
            log_warn!("upnp: delayed SOAP request rejected for {}", action);
        }
    } else {
        if req.send(cb).is_none() {
            log_warn!("upnp: SOAP request rejected for {}", action);
        }
    }
}

fn soap_request(
    url: &str,
    service_urn: &str,
    action: &str,
    args: &[(&str, &str)],
    cb: fn(&FetchResponse),
) {
    soap_request_impl(None, url, service_urn, action, args, cb);
}

fn soap_request_after(
    delay_ms: u32,
    url: &str,
    service_urn: &str,
    action: &str,
    args: &[(&str, &str)],
    cb: fn(&FetchResponse),
) {
    soap_request_impl(Some(delay_ms), url, service_urn, action, args, cb);
}

// ── Parsed state from UPnP responses ────────────────────────────

/// Transport state from `GetTransportInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    Playing,
    Paused,
    Stopped,
    Transitioning,
    NoMedia,
}

impl TransportState {
    fn from_upnp(s: &str) -> Self {
        match s {
            "PLAYING" => Self::Playing,
            "PAUSED_PLAYBACK" => Self::Paused,
            "STOPPED" => Self::Stopped,
            "TRANSITIONING" => Self::Transitioning,
            _ => Self::NoMedia,
        }
    }
}

pub use crate::protocol::TrackMeta;

/// Playhead position with sub-second precision.
///
/// Server status updates carry only integer-second granularity, but the
/// widget interpolates between polls using millisecond frame deltas. A
/// naive `position_secs + delta_ms / 1_000` loses every sub-second
/// remainder, so frame-by-frame the displayed elapsed time drifts. This
/// type carries milliseconds and exposes second-granular getters/setters
/// at the boundaries that need them.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybackPosition {
    ms: u64,
}

impl PlaybackPosition {
    /// Build from a server-reported integer-second value.
    #[must_use]
    pub fn from_secs(secs: u32) -> Self {
        Self {
            ms: u64::from(secs) * 1_000,
        }
    }

    /// Reset to a fresh server-reported integer-second value, clearing
    /// any sub-second remainder accumulated since the last update.
    pub fn set_secs(&mut self, secs: u32) {
        self.ms = u64::from(secs) * 1_000;
    }

    /// Advance the playhead by `delta_ms`, clamped to `max_secs`.
    pub fn advance(&mut self, delta_ms: u32, max_secs: u32) {
        let max_ms = u64::from(max_secs) * 1_000;
        self.ms = (self.ms + u64::from(delta_ms)).min(max_ms);
    }

    /// Current position in whole seconds (truncated).
    #[must_use]
    pub fn as_secs(&self) -> u32 {
        u32::try_from(self.ms / 1_000).unwrap_or(u32::MAX)
    }
}

/// Position info from `GetPositionInfo`.
#[derive(Debug, Clone, Default)]
pub struct PositionInfo {
    pub track_meta: TrackMeta,
    pub playhead: PlaybackPosition,
    pub duration_secs: u32,
}

/// Volume info from `GetVolume` / `GetMute`.
#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeInfo {
    /// 0–1000 (permille). Gives 0.1% precision for Cast's float volume.
    pub level: u32,
    pub muted: bool,
}

// ── Time parsing helpers ─────────────────────────────────────────

/// Parse UPnP duration string `H:MM:SS` or `H:MM:SS.f` into total seconds.
pub fn parse_duration(s: &str) -> u32 {
    let s = s.split('.').next().unwrap_or(s);
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return 0;
    }
    let h: u32 = parts[0].parse().unwrap_or(0);
    let m: u32 = parts[1].parse().unwrap_or(0);
    let s: u32 = parts[2].parse().unwrap_or(0);
    h * 3_600 + m * 60 + s
}

/// Format seconds as `H:MM:SS` or `M:SS`.
pub fn format_duration_hms(total_secs: u32) -> String {
    let h = total_secs / 3_600;
    let m = (total_secs % 3_600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        fmt!("{}:{}:{}", h, zero_pad(m), zero_pad(s))
    } else {
        fmt!("{}:{}", m, zero_pad(s))
    }
}

/// Zero-pad a number to 2 digits.
fn zero_pad(n: u32) -> String {
    if n < 10 {
        fmt!("0{}", n)
    } else {
        fmt!("{}", n)
    }
}

// ── DIDL-Lite metadata parser ────────────────────────────────────

/// Parse DIDL-Lite XML into `TrackMeta`.
///
/// DIDL-Lite is the metadata format used by UPnP for track information.
/// Elements use Dublin Core (`dc:`) and UPnP (`upnp:`) namespaces, but
/// `XmlDoc` matches by local name only, so we query `//title`, `//creator`, etc.
pub fn parse_didl_lite(didl_xml: &str) -> TrackMeta {
    let xml = XmlDoc::parse(didl_xml.as_bytes());
    if !xml.is_valid() {
        return TrackMeta::default();
    }

    let mut fields = Vec::new();
    if let Some(artist) = xml.str("//creator").or_else(|| xml.str("//artist")) {
        fields.push(("Artist".into(), artist));
    }
    if let Some(album) = xml.str("//album") {
        fields.push(("Album".into(), album));
    }

    TrackMeta {
        title: xml.str("//title"),
        fields,
        album_art_uri: xml.str("//albumArtURI"),
    }
}

// ── Response parsers ─────────────────────────────────────────────

/// Parse a `GetPositionInfo` SOAP response.
pub fn parse_position_info(response_body: &[u8]) -> Option<PositionInfo> {
    let xml = XmlDoc::parse(response_body);
    if !xml.is_valid() {
        return None;
    }

    let rel_time = xml.str("//RelTime").unwrap_or_default();
    let track_duration = xml.str("//TrackDuration").unwrap_or_default();
    let track_meta_data = xml.str("//TrackMetaData").unwrap_or_default();

    let track_meta = if track_meta_data.is_empty() || track_meta_data == "NOT_IMPLEMENTED" {
        TrackMeta::default()
    } else {
        // DIDL-Lite is often HTML-entity-encoded inside the SOAP response.
        // XmlDoc::str already returns the decoded text content.
        parse_didl_lite(&track_meta_data)
    };

    Some(PositionInfo {
        track_meta,
        playhead: PlaybackPosition::from_secs(parse_duration(&rel_time)),
        duration_secs: parse_duration(&track_duration),
    })
}

/// Parse a `GetTransportInfo` SOAP response.
pub fn parse_transport_info(response_body: &[u8]) -> Option<TransportState> {
    let xml = XmlDoc::parse(response_body);
    if !xml.is_valid() {
        return None;
    }
    let state_str = xml.str("//CurrentTransportState")?;
    Some(TransportState::from_upnp(&state_str))
}

/// Parse a `GetVolume` SOAP response.
pub fn parse_volume(response_body: &[u8]) -> Option<u32> {
    let xml = XmlDoc::parse(response_body);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    xml.f64("//CurrentVolume").map(|v| v as u32)
}

/// Parse a `GetMute` SOAP response.
pub fn parse_mute(response_body: &[u8]) -> Option<bool> {
    let xml = XmlDoc::parse(response_body);
    let val = xml.str("//CurrentMute")?;
    Some(val == "1" || val == "true")
}

// ── Capability flags ────────────────────────────────────────────

/// Transport capabilities parsed from `GetCurrentTransportActions`.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransportActions {
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

impl Default for TransportActions {
    fn default() -> Self {
        Self {
            can_play: true,
            can_pause: true,
            can_seek: true,
            can_next: true,
            can_previous: true,
        }
    }
}

/// Parse a `GetCurrentTransportActions` SOAP response.
///
/// Returns a comma-separated string like `"Play,Stop,Pause,Seek,Next,Previous"`.
pub fn parse_transport_actions(response_body: &[u8]) -> Option<TransportActions> {
    let xml = XmlDoc::parse(response_body);
    if !xml.is_valid() {
        return None;
    }
    let actions_str = xml.str("//Actions").unwrap_or_default();
    Some(TransportActions {
        can_play: actions_str.contains("Play"),
        can_pause: actions_str.contains("Pause"),
        can_seek: actions_str.contains("Seek"),
        can_next: actions_str.contains("Next"),
        can_previous: actions_str.contains("Previous"),
    })
}

// ── AVTransport actions ──────────────────────────────────────────

/// Send `Play` command.
pub fn play(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "Play",
        &[("InstanceID", "0"), ("Speed", "1")],
        cb,
    );
}

/// Send `Pause` command.
pub fn pause(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "Pause",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Send `Next` track command.
pub fn next(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "Next",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Send `Previous` track command.
pub fn previous(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "Previous",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Send `Seek` to a position (in seconds).
pub fn seek(device: &UpnpDevice, position_secs: u32, cb: fn(&FetchResponse)) {
    let target = format_duration_hms(position_secs);
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "Seek",
        &[
            ("InstanceID", "0"),
            ("Unit", "REL_TIME"),
            ("Target", &target),
        ],
        cb,
    );
}

/// Request `GetPositionInfo` (returns track metadata + current position).
pub fn get_position_info(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "GetPositionInfo",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Request `GetPositionInfo` after a delay (for polling).
pub fn get_position_info_after(delay_ms: u32, device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request_after(
        delay_ms,
        &device.av_transport_url(),
        AV_TRANSPORT,
        "GetPositionInfo",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Request `GetTransportInfo` (returns transport state: playing/paused/stopped).
pub fn get_transport_info(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "GetTransportInfo",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Request `GetTransportInfo` after a delay (for polling).
pub fn get_transport_info_after(delay_ms: u32, device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request_after(
        delay_ms,
        &device.av_transport_url(),
        AV_TRANSPORT,
        "GetTransportInfo",
        &[("InstanceID", "0")],
        cb,
    );
}

/// Request `GetCurrentTransportActions` (returns which actions are currently valid).
pub fn get_transport_actions(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.av_transport_url(),
        AV_TRANSPORT,
        "GetCurrentTransportActions",
        &[("InstanceID", "0")],
        cb,
    );
}

// ── RenderingControl actions ─────────────────────────────────────

/// Request current volume level (0–100).
pub fn get_volume(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.rendering_control_url(),
        RENDERING_CONTROL,
        "GetVolume",
        &[("InstanceID", "0"), ("Channel", "Master")],
        cb,
    );
}

/// Set volume level (0–100).
pub fn set_volume(device: &UpnpDevice, level: u32, cb: fn(&FetchResponse)) {
    let level_str = fmt!("{}", level.min(100));
    soap_request(
        &device.rendering_control_url(),
        RENDERING_CONTROL,
        "SetVolume",
        &[
            ("InstanceID", "0"),
            ("Channel", "Master"),
            ("DesiredVolume", &level_str),
        ],
        cb,
    );
}

/// Request mute state.
pub fn get_mute(device: &UpnpDevice, cb: fn(&FetchResponse)) {
    soap_request(
        &device.rendering_control_url(),
        RENDERING_CONTROL,
        "GetMute",
        &[("InstanceID", "0"), ("Channel", "Master")],
        cb,
    );
}

/// Set mute state.
pub fn set_mute(device: &UpnpDevice, muted: bool, cb: fn(&FetchResponse)) {
    let val = if muted { "1" } else { "0" };
    soap_request(
        &device.rendering_control_url(),
        RENDERING_CONTROL,
        "SetMute",
        &[
            ("InstanceID", "0"),
            ("Channel", "Master"),
            ("DesiredMute", val),
        ],
        cb,
    );
}
