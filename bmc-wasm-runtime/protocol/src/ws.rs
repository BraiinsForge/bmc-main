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

//! Why the host ended a WebSocket, as it crosses the host/widget seam.
//!
//! A close the peer sent carries the peer's code. One the host chose arrives
//! as its own event carrying a [`WsHostClose`], so a peer's 1008 or 1012
//! is never taken for the host's decision.
//!
//! Wire format: one byte. `from_wire` rejects every value this build cannot name,
//! so a reason added later is never mistaken for another.

/// The kind of event `__on_ws_event` delivers, as its `event_type` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsEventKind {
    /// The handshake finished. No data.
    Open,
    /// A text message. Data: its bytes, which the widget checks are UTF-8.
    Text,
    /// The peer closed the socket, or it dropped. Data: the close code, two bytes little-endian.
    Close,
    /// The host closed the socket. Data: one byte, a [`WsHostClose`].
    HostClosed,
}

impl WsEventKind {
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Open => 0,
            Self::Text => 1,
            Self::Close => 2,
            Self::HostClosed => 3,
        }
    }

    /// `None` for a kind this build has no meaning for.
    #[must_use]
    pub const fn from_wire(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Open),
            1 => Some(Self::Text),
            2 => Some(Self::Close),
            3 => Some(Self::HostClosed),
            _ => None,
        }
    }
}

/// Why the host closed a WebSocket, or never opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum WsHostClose {
    /// The URL or a message spends a credential the host will not send there,
    /// such as one whose slot is unbound. Retrying is futile until the credentials change.
    Credential,
    /// The account behind a credential the socket spent changed.
    /// A new socket spends the account as it now is.
    AccountChanged,
    /// The peer sent a message larger than the host accepts.
    MessageTooLarge,
}

/// The close code a widget older than [`WS_HOST_CLOSE_PROTOCOL`](crate::version::WS_HOST_CLOSE_PROTOCOL)
/// gets for every [`WsHostClose`]: the abnormal closure such a widget has always seen
/// for a socket that ended without the peer's close.
pub const LEGACY_HOST_CLOSE_CODE: u16 = 1006;

impl WsHostClose {
    /// Never 0, so a zeroed field cannot read as a reason.
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        match self {
            Self::Credential => 1,
            Self::AccountChanged => 2,
            Self::MessageTooLarge => 3,
        }
    }

    /// `None` for a reason this build has no meaning for.
    #[must_use]
    pub const fn from_wire(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Credential),
            2 => Some(Self::AccountChanged),
            3 => Some(Self::MessageTooLarge),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WsEventKind, WsHostClose};

    #[test]
    fn round_trips_every_event_kind() {
        for kind in [
            WsEventKind::Open,
            WsEventKind::Text,
            WsEventKind::Close,
            WsEventKind::HostClosed,
        ] {
            assert_eq!(WsEventKind::from_wire(kind.to_wire()), Some(kind));
        }
        assert_eq!(WsEventKind::from_wire(4), None);
    }

    #[test]
    fn round_trips_every_reason() {
        for reason in [
            WsHostClose::Credential,
            WsHostClose::AccountChanged,
            WsHostClose::MessageTooLarge,
        ] {
            assert_eq!(WsHostClose::from_wire(reason.to_wire()), Some(reason));
        }
    }

    #[test]
    fn an_unnamed_value_is_no_reason() {
        assert_eq!(WsHostClose::from_wire(0), None);
        assert_eq!(WsHostClose::from_wire(4), None);
    }
}
