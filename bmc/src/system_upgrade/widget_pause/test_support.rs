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

use crate::system_upgrade::WidgetLifecycle;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Stop,
    Restart,
}

#[derive(Debug)]
pub(crate) enum StopBehaviour {
    Immediate,
    /// Each stop waits for one [`ScriptedLifecycle::release_stop`].
    Held,
    Panics,
}

#[derive(Debug)]
enum RestartBehaviour {
    Immediate,
    /// Each restart waits for one [`ScriptedLifecycle::release_restart`].
    Held,
}

#[derive(Debug)]
pub(crate) struct ScriptedLifecycle {
    stop: StopBehaviour,
    restart: RestartBehaviour,
    stop_released: tokio::sync::Notify,
    restart_released: tokio::sync::Notify,
    calls: Mutex<Vec<Call>>,
}

impl ScriptedLifecycle {
    pub(crate) fn new(stop: StopBehaviour) -> Arc<Self> {
        Self::build(stop, RestartBehaviour::Immediate)
    }

    pub(crate) fn holding_restarts(stop: StopBehaviour) -> Arc<Self> {
        Self::build(stop, RestartBehaviour::Held)
    }

    fn build(stop: StopBehaviour, restart: RestartBehaviour) -> Arc<Self> {
        Arc::new(Self {
            stop,
            restart,
            stop_released: tokio::sync::Notify::new(),
            restart_released: tokio::sync::Notify::new(),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("BUG: call log poisoned").clone()
    }

    pub(crate) fn release_stop(&self) {
        self.stop_released.notify_one();
    }

    pub(crate) fn release_restart(&self) {
        self.restart_released.notify_one();
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .expect("BUG: call log poisoned")
            .push(call);
    }
}

#[async_trait::async_trait]
impl WidgetLifecycle for ScriptedLifecycle {
    async fn stop_all_widgets(&self) {
        self.record(Call::Stop);
        match self.stop {
            StopBehaviour::Immediate => {}
            StopBehaviour::Held => self.stop_released.notified().await,
            StopBehaviour::Panics => panic!("scripted widget stop failure"),
        }
    }

    async fn restart_widgets(&self) {
        self.record(Call::Restart);
        match self.restart {
            RestartBehaviour::Immediate => {}
            RestartBehaviour::Held => self.restart_released.notified().await,
        }
    }

    async fn refresh_widgets(&self) {
        unreachable!("BUG: a firmware pause never refreshes widgets");
    }
}

/// Runs every task until all of them wait: paused test time only advances
/// once the runtime is idle.
pub(crate) async fn settle() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}
