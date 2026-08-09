// Copyright (C) 2025  Braiins Systems s.r.o.
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

use bmc_scheduler::{Cron, jobs::BoxedTask};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::time::{Duration, Instant};
use tracing::debug;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UpgradeStatus {
    NotStarted,
    DownloadReady,
    InProgress,
    Success,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
pub struct AutoUpgradeConfig {
    pub enabled: bool,
    /// Written for firmware rollback only.
    /// An older BMC schedules from this cron; current code
    /// derives from the maintenance stagger and never reads it.
    pub cron: Option<Cron>,
}

#[derive(Clone)]
pub struct AutoUpgrade {
    pub task: Arc<BoxedTask>,
    pub notifier: Arc<Notify>,
}

impl Debug for AutoUpgrade {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoUpgrade")
            .field("notifier", &self.notifier)
            .field("task", &self.notifier) // Intentionally not printing the task
            .finish()
    }
}

impl AutoUpgrade {
    pub const AUTOUPGRADE_SOURCE_NAME: &str = "autoupgrade";

    #[must_use]
    pub fn new(notifier: Notify, start_time: Instant, minimum_uptime: Duration) -> Self {
        let notifier = Arc::new(notifier);
        let task = {
            let notifier_clone = notifier.clone();
            move || Self::autoupgrade_task(notifier_clone.clone(), start_time, minimum_uptime)
        };
        let task: BoxedTask = Box::new(task);
        Self {
            task: Arc::new(task),
            notifier,
        }
    }

    fn autoupgrade_task(
        sender: Arc<Notify>,
        start_time: Instant,
        minimum_uptime: Duration,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            if start_time.elapsed() < minimum_uptime {
                debug!("Skipping the auto-upgrade tick inside the startup window");
                return;
            }

            // `notify_one` stores a permit when nobody waits, so a tick landing
            // while the consumer is busy coalesces into one pending run instead
            // of vanishing (`notify_waiters` would drop it).
            sender.notify_one();
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::FutureExt as _;

    const TEST_MINIMUM_UPTIME: Duration = Duration::from_secs(1);

    #[tokio::test(start_paused = true)]
    async fn a_tick_inside_the_boot_window_does_not_notify() {
        let auto = AutoUpgrade::new(
            Notify::new(),
            tokio::time::Instant::now(),
            TEST_MINIMUM_UPTIME,
        );
        (auto.task)().await;
        assert!(
            auto.notifier.notified().now_or_never().is_none(),
            "the floor must hold the tick back during the boot window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_tick_past_the_boot_window_notifies_once() {
        let auto = AutoUpgrade::new(
            Notify::new(),
            tokio::time::Instant::now(),
            TEST_MINIMUM_UPTIME,
        );
        tokio::time::advance(TEST_MINIMUM_UPTIME).await;
        (auto.task)().await;
        assert!(auto.notifier.notified().now_or_never().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn ticks_during_a_busy_consumer_coalesce_to_one_permit() {
        let auto = AutoUpgrade::new(
            Notify::new(),
            tokio::time::Instant::now(),
            TEST_MINIMUM_UPTIME,
        );
        tokio::time::advance(TEST_MINIMUM_UPTIME).await;
        (auto.task)().await;
        (auto.task)().await;
        assert!(
            auto.notifier.notified().now_or_never().is_some(),
            "the first pending tick must be delivered"
        );
        assert!(
            auto.notifier.notified().now_or_never().is_none(),
            "a second tick during a busy consumer must coalesce, not queue"
        );
    }
}
