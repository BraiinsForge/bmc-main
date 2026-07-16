// Copyright (C) 2025  Braiins Systems s.r.o.
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

use futures::{FutureExt, future::select_all};
use tokio::signal::unix::{Signal, SignalKind, signal};

pub const SHUTDOWN_SIGNALS: &[SignalKind] = &[
    SignalKind::interrupt(),
    SignalKind::terminate(),
    SignalKind::quit(),
];

pub async fn wait_for_first_signal(signals: &[SignalKind]) -> SignalKind {
    let mut signal_streams: Vec<Signal> = signals
        .iter()
        .map(|&sig| {
            signal(sig).unwrap_or_else(|_| panic!("BUG: cannot create signal receiver for {sig:?}"))
        })
        .collect();

    let receivers: Vec<_> = signal_streams
        .iter_mut()
        .map(|sig| sig.recv().boxed())
        .collect();

    let (_, index, _) = select_all(receivers).await;

    signals[index]
}
