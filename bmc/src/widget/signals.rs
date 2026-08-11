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

use std::io;
use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::config::ConfigHandle;

use super::Coordinator;

pub(crate) fn spawn_reload_signal_task(
    coordinator: Arc<Coordinator>,
    config_handle: Arc<RwLock<ConfigHandle>>,
) -> io::Result<JoinHandle<()>> {
    let mut winch = signal(SignalKind::window_change())?;
    let mut usr1 = signal(SignalKind::user_defined1())?;
    let mut usr2 = signal(SignalKind::user_defined2())?;

    Ok(tokio::spawn(async move {
        loop {
            tokio::select! {
                signal = winch.recv() => {
                    if signal.is_none() {
                        warn!("SIGWINCH stream closed; widget reload task exiting");
                        return;
                    }
                    coordinator.reload_changed_widgets(&config_handle).await;
                }
                signal = usr1.recv() => {
                    if signal.is_none() {
                        warn!("SIGUSR1 stream closed; widget reload task exiting");
                        return;
                    }
                }
                signal = usr2.recv() => {
                    if signal.is_none() {
                        warn!("SIGUSR2 stream closed; widget reload task exiting");
                        return;
                    }
                }
            }
        }
    }))
}
