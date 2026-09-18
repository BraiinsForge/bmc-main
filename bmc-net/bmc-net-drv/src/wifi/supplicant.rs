// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

//! Why a station join failed, taken from wpa_supplicant itself.
//!
//! The join helpers can only observe that a station never associated or never
//! got an address; the reason - a rejected passphrase, an SSID that is not on
//! the air - is known only to the supplicant. It publishes it on a per-device
//! control socket, which this module attaches to for the duration of a join so
//! a failure can be reported as something the operator can act on.

use std::os::unix::fs::{MetadataExt as _, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use bmc_net_types::wifi::WifiJoinError;
use log::debug;
use tokio::net::UnixDatagram;
use tokio::time::{Duration, timeout};

/// Where wpa_supplicant keeps one control socket per WiFi device.
const CONTROL_SOCKET_DIR: &str = "/var/run/wpa_supplicant";
/// Our end of the socket pair; the supplicant sends unsolicited events here.
const CLIENT_SOCKET_DIR: &str = "/tmp";
/// Largest control message the supplicant sends.
const EVENT_BUFFER_LEN: usize = 4096;
/// How long draining waits for the next event before calling the socket idle.
///
/// The events are already queued on the socket: this only covers the wakeup,
/// and a join polls once a second, so it costs nothing to be generous.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);
/// Most events one drain takes, so a talkative supplicant cannot keep the join
/// out of its own checks.
const DRAIN_LIMIT: usize = 64;
/// Most events one join keeps. A verdict needs the last few, not the history.
const EVENT_LIMIT: usize = 256;
/// How long a control command waits for its reply.
///
/// A join must not hang on the diagnosis of a join: a supplicant that does not
/// answer leaves the failure unexplained, which is what it was before.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
/// How many times a join tries to subscribe before it stops asking.
///
/// Only a supplicant that is there and unresponsive counts: each such attempt
/// costs [`REPLY_TIMEOUT`], and the join polls once a second. A control socket
/// that has not appeared yet is the normal state early in a join and is free
/// to retry.
const ATTACH_ATTEMPTS: u8 = 3;

/// Distinguishes the client sockets of joins that overlap, so one join's
/// subscription cannot unlink another's.
static CLIENT_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// A wpa_supplicant control connection subscribed to the event stream of one
/// device.
///
/// Dropping it detaches and removes the client socket, so a join that ends
/// early leaves nothing behind in `/tmp`.
#[derive(Debug)]
struct JoinWatcher {
    socket: UnixDatagram,
    client_path: PathBuf,
    server_path: PathBuf,
    /// The netdev the subscription is for. An ESP32 reload can rename it
    /// while the old control socket lingers, so the name is checked as well
    /// as the socket identity.
    device: String,
    /// Identifies the supplicant instance we subscribed to. Applying a station
    /// config restarts it, and the new instance knows nothing of our
    /// subscription, so a join has to notice and subscribe again.
    server_inode: u64,
}

impl JoinWatcher {
    /// Subscribes to `device`'s event stream, or fails when the supplicant is
    /// not managing it (yet).
    async fn attach(device: &str, events: &mut Vec<String>) -> Result<Self> {
        let server_path = control_socket(device);
        let client_path = Path::new(CLIENT_SOCKET_DIR).join(format!(
            "bmc-wpa-{}-{}-{device}",
            std::process::id(),
            CLIENT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        // A leftover from a killed process would make the bind fail.
        let _ = std::fs::remove_file(&client_path);

        let socket = UnixDatagram::bind(&client_path)
            .with_context(|| format!("binding {}", client_path.display()))?;
        // Owned from here on: whatever fails below, `Drop` unlinks the path.
        let mut watcher = Self {
            socket,
            client_path,
            server_path,
            device: device.to_owned(),
            server_inode: 0,
        };
        watcher.connect()?;
        watcher.request("ATTACH", events).await?;
        Ok(watcher)
    }

    fn connect(&mut self) -> Result<()> {
        // The supplicant runs as its own user and answers to this address, so
        // it has to be able to write to it. Without this the ATTACH reply never
        // arrives and every join would wait for a diagnosis that cannot come.
        // No exec bit: it is a socket, and anything that can write here can
        // only feed us events we attribute to this device.
        std::fs::set_permissions(&self.client_path, std::fs::Permissions::from_mode(0o666))
            .with_context(|| format!("opening {} to wpa_supplicant", self.client_path.display()))?;
        self.socket
            .connect(&self.server_path)
            .with_context(|| format!("connecting to {}", self.server_path.display()))?;
        self.server_inode = inode(&self.server_path)
            .with_context(|| format!("reading {}", self.server_path.display()))?;
        Ok(())
    }

    /// Sends a control command and waits for its reply, keeping any event that
    /// arrives in the meantime.
    async fn request(&mut self, command: &str, events: &mut Vec<String>) -> Result<()> {
        timeout(REPLY_TIMEOUT, self.exchange(command, events))
            .await
            .map_err(|_| anyhow!("wpa_supplicant did not answer {command}"))?
    }

    /// One command/reply round trip, unbounded on its own.
    async fn exchange(&mut self, command: &str, events: &mut Vec<String>) -> Result<()> {
        self.socket
            .send(command.as_bytes())
            .await
            .with_context(|| format!("sending {command} to wpa_supplicant"))?;
        let mut buffer = [0_u8; EVENT_BUFFER_LEN];
        // The reply can queue behind events, which are kept rather than dropped.
        for _ in 0..DRAIN_LIMIT {
            let read = self.socket.recv(&mut buffer).await?;
            let message = String::from_utf8_lossy(&buffer[..read]).into_owned();
            if message.starts_with('<') {
                push_event(events, message);
                continue;
            }
            return match message.trim() {
                "OK" => Ok(()),
                other => bail!("wpa_supplicant rejected {command}: {other}"),
            };
        }
        bail!("only events, no reply to {command} from wpa_supplicant")
    }

    /// Collects the events that arrived since the last call.
    ///
    /// Reading with a short timeout rather than `try_recv`: the latter answers
    /// from the readiness registration, which nothing here ever arms, so it
    /// would report an empty socket however many events are queued on it.
    async fn poll_events(&mut self, events: &mut Vec<String>) {
        let mut buffer = [0_u8; EVENT_BUFFER_LEN];
        for _ in 0..DRAIN_LIMIT {
            let Ok(Ok(read)) = timeout(DRAIN_TIMEOUT, self.socket.recv(&mut buffer)).await else {
                return;
            };
            let message = String::from_utf8_lossy(&buffer[..read]).into_owned();
            debug!("wpa_supplicant: {}", message.trim());
            push_event(events, message);
        }
    }

    /// Whether the supplicant we subscribed to has been replaced, or the
    /// station now goes by another name.
    fn server_replaced(&self, device: &str) -> bool {
        self.device != device
            || inode(&self.server_path).is_none_or(|current| current != self.server_inode)
    }
}

impl Drop for JoinWatcher {
    fn drop(&mut self) {
        // Best effort: the supplicant drops an unresponsive subscriber anyway.
        let _ = self.socket.try_send(b"DETACH");
        let _ = std::fs::remove_file(&self.client_path);
    }
}

/// Follows one join: subscribes when the supplicant appears, subscribes again
/// when it is replaced, and stops trying after [`ATTACH_ATTEMPTS`] failed
/// attempts.
///
/// The events live here rather than in the subscription, so they outlive a
/// drained socket while the watcher stays current. Once the supplicant is
/// replaced they are discarded: they described the configuration the old
/// instance ran with, and the verdict rests on what the new one says.
#[derive(Debug)]
pub(crate) struct JoinDiagnosis {
    watcher: Option<JoinWatcher>,
    events: Vec<String>,
    attempts_left: u8,
}

impl Default for JoinDiagnosis {
    fn default() -> Self {
        Self {
            watcher: None,
            events: Vec::new(),
            attempts_left: ATTACH_ATTEMPTS,
        }
    }
}

impl JoinDiagnosis {
    /// Drains what the supplicant has said, subscribing or re-subscribing if
    /// that is still worth a try. Called between the join's own attempts.
    pub(crate) async fn poll(&mut self, device: &str) {
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.poll_events(&mut self.events).await;
            if !watcher.server_replaced(device) {
                return;
            }
            // What the old instance said was about the configuration it ran
            // with; the verdict rests on the new one.
            debug!("wpa_supplicant restarted, subscribing to the new one");
            self.watcher = None;
            self.events.clear();
        }
        if self.attempts_left == 0 || !control_socket(device).exists() {
            return;
        }
        match JoinWatcher::attach(device, &mut self.events).await {
            Ok(watcher) => self.watcher = Some(watcher),
            Err(e) => {
                self.attempts_left = self.attempts_left.saturating_sub(1);
                debug!(
                    "wpa_supplicant did not take the subscription ({} attempts left): {e}",
                    self.attempts_left
                );
            }
        }
    }

    /// Forgets what the supplicant said before the station associated.
    ///
    /// A scan that had not yet found the network says nothing about a join
    /// that reached the access point and then failed to get an address.
    pub(crate) fn associated(&mut self) {
        self.events.clear();
    }

    /// The reason the supplicant gave for failing to join `ssid`, if it gave one.
    pub(crate) async fn verdict(&mut self, ssid: &str) -> Option<WifiJoinError> {
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.poll_events(&mut self.events).await;
        }
        classify(self.events.iter().map(String::as_str), ssid)
    }
}

/// Keeps the newest events and drops the rest: a verdict needs the tail of the
/// stream, and a join can run for a minute against a flapping access point.
fn push_event(events: &mut Vec<String>, message: String) {
    if events.len() >= EVENT_LIMIT {
        events.remove(0);
    }
    events.push(message);
}

/// The control socket wpa_supplicant opens for `device`.
fn control_socket(device: &str) -> PathBuf {
    Path::new(CONTROL_SOCKET_DIR).join(device)
}

/// Inode of a control socket, `None` while it does not exist.
fn inode(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.ino())
}

/// The network an event is about, when it names one.
/// The SSID an event names, as bytes: `ssid="..."` in the CTRL events and
/// `SSID='...'` in the SME lines. Both go through the supplicant's
/// `printf_encode`, so the field is decoded before anyone compares it.
/// One line of a supplicant event taken apart: what it reports, the SSID it
/// names, and the text around that SSID. The SSID is a payload the network
/// chose, so nothing is ever looked for inside it.
struct Line<'a> {
    /// The first word after the `<N>` priority: `CTRL-EVENT-...`, `SME:`,
    /// `WPA:`, `Associated`...
    kind: &'a str,
    ssid: Option<Vec<u8>>,
    /// The line with the SSID cut out, where the fields are.
    outside: String,
}

impl<'a> Line<'a> {
    fn parse(line: &'a str) -> Self {
        let body = line
            .strip_prefix('<')
            .and_then(|rest| rest.split_once('>'))
            .map_or(line, |(_, body)| body);
        let kind = body.split_whitespace().next().unwrap_or_default();
        let Some((ssid, range)) = ssid_span(body) else {
            return Self {
                kind,
                ssid: None,
                outside: body.to_owned(),
            };
        };
        let outside = format!(
            "{}{}",
            body.get(..range.start).unwrap_or_default(),
            body.get(range.end..).unwrap_or_default()
        );
        Self {
            kind,
            ssid: Some(ssid),
            outside,
        }
    }

    /// Whether a `key=value` field outside the SSID reads exactly `field`.
    fn has_field(&self, field: &str) -> bool {
        self.outside.split_whitespace().any(|word| word == field)
    }
}

/// The SSID `body` names, decoded, and where its encoded form sits: `ssid="..."`
/// in the CTRL events, `SSID='...'` in the SME lines. Whichever marker comes
/// first is the real one: a payload can only follow its own marker, so text
/// inside the SSID that looks like the other marker is never mistaken for it.
fn ssid_span(body: &str) -> Option<(Vec<u8>, std::ops::Range<usize>)> {
    const CTRL_MARKER: &str = "ssid=\"";
    const SME_MARKER: &str = "SSID='";
    let ctrl = body.find(CTRL_MARKER);
    let sme = body.find(SME_MARKER);
    let ctrl_first = match (ctrl, sme) {
        (Some(ctrl), Some(sme)) => ctrl < sme,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => return None,
    };
    if ctrl_first {
        let start = ctrl? + CTRL_MARKER.len();
        let payload = quoted(body.get(start..).unwrap_or_default());
        return Some((decode_printf(payload), start..start + payload.len()));
    }
    let start = sme? + SME_MARKER.len();
    let rest = body.get(start..).unwrap_or_default();
    // The encoder leaves `'` alone, so the payload may hold quotes of its own.
    // The SME line always follows the SSID with `' freq=`, and the real one
    // is the last; a line without it ends the field at its last quote.
    let len = rest
        .rfind("' freq=")
        .or_else(|| rest.rfind('\''))
        .unwrap_or(rest.len());
    let payload = rest.get(..len).unwrap_or(rest);
    Some((decode_printf(payload), start..start + len))
}

/// The text up to the closing quote, escaped quotes skipped.
fn quoted(rest: &str) -> &str {
    let mut escaped = false;
    for (index, byte) in rest.bytes().enumerate() {
        match byte {
            b'\\' if !escaped => escaped = true,
            b'"' if !escaped => return rest.get(..index).unwrap_or(rest),
            _ => escaped = false,
        }
    }
    rest
}

/// Undoes wpa_supplicant's `printf_encode`: `\xNN` for a byte outside
/// printable ASCII, `\"`, `\\`, `\e`, `\n`, `\r` and `\t` for the rest.
fn decode_printf(field: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut bytes = field.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        match bytes.next() {
            Some(b'x') => {
                let hex: String = bytes.by_ref().take(2).map(char::from).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(value) => out.push(value),
                    // Not an escape after all: keep what was read.
                    Err(_) => out.extend_from_slice(format!("\\x{hex}").as_bytes()),
                }
            }
            Some(b'e') => out.push(0x1b),
            Some(b'n') => out.push(b'\n'),
            Some(b'r') => out.push(b'\r'),
            Some(b't') => out.push(b'\t'),
            Some(other) => out.push(other),
            None => out.push(b'\\'),
        }
    }
    out
}

/// What the events say about a join of `ssid`, read in the order they came.
///
/// A rejected passphrase names the network and is final. A scan that did not
/// find the network is only the verdict while nothing later shows the access
/// point answering: the supplicant tries again, and an authentication attempt
/// for `ssid` proves it is on the air, so the failure is then something else.
fn classify<'a>(events: impl Iterator<Item = &'a str>, ssid: &str) -> Option<WifiJoinError> {
    let mut not_found = false;
    for event in events {
        for line in event.lines() {
            let line = Line::parse(line);
            if line
                .ssid
                .as_deref()
                .is_some_and(|named| named != ssid.as_bytes())
            {
                continue;
            }
            if line.has_field("reason=WRONG_KEY") {
                return Some(WifiJoinError::WrongKey(ssid.to_owned()));
            }
            if line.kind == "CTRL-EVENT-NETWORK-NOT-FOUND" {
                not_found = true;
            } else if line.ssid.is_some() {
                not_found = false;
            }
        }
    }
    not_found.then(|| WifiJoinError::NetworkNotFound(ssid.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines as a BMM101 emitted them, `logread` timestamps stripped.
    const WRONG_KEY: &[&str] = &[
        "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='I301' freq=2462 MHz)",
        "<3>Associated with c0:06:c3:ec:3f:06",
        "<3>CTRL-EVENT-DISCONNECTED bssid=c0:06:c3:ec:3f:06 reason=15",
        "<3>WPA: 4-Way Handshake failed - pre-shared key may be incorrect",
        "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"I301\" auth_failures=1 duration=10 reason=WRONG_KEY",
    ];

    const NOT_FOUND: &[&str] = &[
        "<3>CTRL-EVENT-SCAN-STARTED ",
        "<3>CTRL-EVENT-SCAN-RESULTS ",
        "<3>CTRL-EVENT-NETWORK-NOT-FOUND ",
    ];

    #[test]
    fn a_rejected_passphrase_is_recognised() {
        assert!(matches!(
            classify(WRONG_KEY.iter().copied(), "I301"),
            Some(WifiJoinError::WrongKey(ssid)) if ssid == "I301"
        ));
    }

    #[test]
    fn a_network_that_is_not_on_air_is_recognised() {
        assert!(matches!(
            classify(NOT_FOUND.iter().copied(), "Absent"),
            Some(WifiJoinError::NetworkNotFound(ssid)) if ssid == "Absent"
        ));
    }

    #[test]
    fn a_passphrase_rejection_wins_over_a_later_scan_failure() {
        let events = WRONG_KEY.iter().copied().chain(NOT_FOUND.iter().copied());
        assert!(matches!(
            classify(events, "I301"),
            Some(WifiJoinError::WrongKey(_))
        ));
    }

    #[test]
    fn a_join_in_progress_has_no_verdict() {
        let events = [
            "<3>CTRL-EVENT-SCAN-STARTED ",
            "<3>Associated with c0:06:c3:ec:3f:06",
        ];
        assert!(classify(events.iter().copied(), "I301").is_none());
    }

    #[test]
    fn a_rejection_meant_for_another_network_is_not_ours() {
        assert!(classify(WRONG_KEY.iter().copied(), "Another").is_none());
    }

    #[test]
    fn a_not_found_before_the_network_answered_is_not_the_verdict() {
        let events = NOT_FOUND.iter().copied().chain([
            "<3>CTRL-EVENT-SCAN-STARTED ",
            "<3>CTRL-EVENT-SCAN-RESULTS ",
            "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='I301' freq=2462 MHz)",
            "<3>CTRL-EVENT-DISCONNECTED bssid=c0:06:c3:ec:3f:06 reason=3",
        ]);
        assert!(classify(events, "I301").is_none());
    }

    #[test]
    fn a_not_found_after_the_network_went_away_again_stands() {
        let events = [
            "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='I301' freq=2462 MHz)",
            "<3>CTRL-EVENT-DISCONNECTED bssid=c0:06:c3:ec:3f:06 reason=3",
        ]
        .into_iter()
        .chain(NOT_FOUND.iter().copied());
        assert!(matches!(
            classify(events, "I301"),
            Some(WifiJoinError::NetworkNotFound(_))
        ));
    }

    #[test]
    fn a_non_ascii_ssid_is_matched_through_the_supplicants_encoding() {
        let events = [
            r"<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='Dom\xc5\xaf' freq=2462 MHz)",
            "<3>WPA: 4-Way Handshake failed - pre-shared key may be incorrect",
            r#"<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid="Dom\xc5\xaf" auth_failures=1 duration=10 reason=WRONG_KEY"#,
        ];
        assert!(matches!(
            classify(events.iter().copied(), "Domů"),
            Some(WifiJoinError::WrongKey(ssid)) if ssid == "Domů"
        ));
        assert!(classify(events.iter().copied(), "Domu").is_none());
    }

    #[test]
    fn a_quoted_ssid_is_matched_through_the_supplicants_encoding() {
        let events = [
            r#"<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid="Say \"hi\" \\ done" auth_failures=1 duration=10 reason=WRONG_KEY"#,
        ];
        assert!(matches!(
            classify(events.iter().copied(), r#"Say "hi" \ done"#),
            Some(WifiJoinError::WrongKey(_))
        ));
        assert!(classify(events.iter().copied(), "Say hi").is_none());
    }

    #[test]
    fn a_token_shaped_ssid_cannot_forge_a_verdict() {
        let wrong_key = [
            "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='reason=WRONG_KEY' freq=2462 MHz)",
        ];
        assert!(classify(wrong_key.iter().copied(), "reason=WRONG_KEY").is_none());

        let not_found = [
            "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='CTRL-EVENT-NETWORK-NOT-FOUND' freq=2462 MHz)",
        ];
        assert!(classify(not_found.iter().copied(), "CTRL-EVENT-NETWORK-NOT-FOUND").is_none());

        // The genuine verdict for such a network still comes through.
        let rejected = [
            r#"<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid="reason=WRONG_KEY" auth_failures=1 duration=10 reason=WRONG_KEY"#,
        ];
        assert!(matches!(
            classify(rejected.iter().copied(), "reason=WRONG_KEY"),
            Some(WifiJoinError::WrongKey(_))
        ));
    }

    #[test]
    fn an_apostrophe_in_the_ssid_does_not_end_the_sme_field() {
        let events = NOT_FOUND.iter().copied().chain([
            "<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='James' WiFi' freq=2462 MHz)",
            "<3>CTRL-EVENT-DISCONNECTED bssid=c0:06:c3:ec:3f:06 reason=3",
        ]);
        assert!(classify(events, "James' WiFi").is_none());
        assert_eq!(
            Line::parse("<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='James' WiFi' freq=2462 MHz)").ssid,
            Some(b"James' WiFi".to_vec())
        );
    }

    #[test]
    fn a_marker_inside_the_ssid_does_not_move_the_field() {
        let sme = Line::parse(
            r#"<3>SME: Trying to authenticate with c0:06:c3:ec:3f:06 (SSID='ssid="x' freq=2462 MHz)"#,
        );
        assert_eq!(sme.ssid, Some(br#"ssid="x"#.to_vec()));
        let ctrl = Line::parse(
            r#"<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid="SSID='y" auth_failures=1 duration=10 reason=WRONG_KEY"#,
        );
        assert_eq!(ctrl.ssid, Some(b"SSID='y".to_vec()));
        assert!(ctrl.has_field("reason=WRONG_KEY"));
    }

    #[test]
    fn the_encoding_round_trips_the_control_characters() {
        assert_eq!(decode_printf(r"a\tb\nc\rd\ee"), b"a\tb\nc\rd\x1be");
        assert_eq!(decode_printf(r"tail\"), b"tail\\");
        assert_eq!(decode_printf(r"\xzz"), b"\\xzz");
    }

    #[test]
    fn the_newest_events_survive_the_cap() {
        let mut events = Vec::new();
        for i in 0..EVENT_LIMIT + 10 {
            push_event(&mut events, format!("<3>CTRL-EVENT-SCAN-STARTED {i}"));
        }
        assert_eq!(events.len(), EVENT_LIMIT);
        assert!(
            events
                .last()
                .is_some_and(|last| last.ends_with(&format!("{}", EVENT_LIMIT + 9)))
        );
    }
}
