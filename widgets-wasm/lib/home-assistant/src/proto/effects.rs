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

//! What the session asks its caller to do with the socket and the timer,
//! and what the caller tells it back.

use super::reply::Event;

/// Something for the caller to do with its socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Open a socket to `url`, the [`Setup::websocket_url`](super::Setup::websocket_url).
    /// A refused open is reported like any close, with [`Session::on_close`](super::Session::on_close).
    Connect {
        url: String,
    },
    Send(String),
    /// Close the socket. The core expects no close report for it.
    Close,
}

/// How the caller's socket ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    /// The peer closed it, or the connection failed.
    Dropped,
    /// The host refused the credential the connect or a message spends.
    CredentialRefused,
    /// The host closed it because the account behind the credential changed.
    AccountChanged,
    /// The host closed it over a message larger than it accepts.
    MessageTooLarge,
    /// The host refused to open it: it cannot dial the address's scheme, such as `wss`.
    UnsupportedScheme,
    /// The host refused to open it: the address does not parse.
    InvalidRequest,
}

/// When the caller's timer should next fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// This many seconds from now, replacing any earlier request.
    In(u32),
    /// Not until something else happens.
    Never,
}

/// What the caller must do with its socket and timer after one [`Session`](super::Session) call.
#[derive(Debug, Default, PartialEq)]
pub struct Effects {
    pub commands: Vec<Command>,
    /// `None` leaves the timer as it was.
    pub wake: Option<Wake>,
}

impl Effects {
    pub(super) fn send(message: String) -> Self {
        Self {
            commands: vec![Command::Send(message)],
            wake: None,
        }
    }
}

/// What the caller must do after the socket or the timer reported something:
/// carry out the effects, then hand the widget the events.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    pub effects: Effects,
    pub events: Vec<Event>,
}
