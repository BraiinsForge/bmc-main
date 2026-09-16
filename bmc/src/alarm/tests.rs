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

use super::*;

mod backend;
mod controller;
mod next_alarm;
mod schedule;
mod snooze;

struct AlarmFixture {
    temp: tempfile::TempDir,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    config_path: std::path::PathBuf,
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: JobScheduler,
    bus: AlarmBus,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
}

enum CrontabSetup {
    Writable,
    Unwritable,
}

impl AlarmFixture {
    async fn new(product: bmc_platform::Product) -> Self {
        Self::build(product, CrontabSetup::Writable).await
    }

    /// A fixture whose crontab directory is a regular file,
    /// so the scheduler cannot create its crontab and `cancel` fails.
    /// Do not instead make the crontab path a directory:
    /// `Crontab::read_from_stream` discards read errors,
    /// and reading a directory yields EISDIR forever, so the scheduler spins.
    async fn with_unwritable_crontab(product: bmc_platform::Product) -> Self {
        Self::build(product, CrontabSetup::Unwritable).await
    }

    async fn build(product: bmc_platform::Product, crontab_setup: CrontabSetup) -> Self {
        let temp = tempfile::tempdir().expect("BUG: create alarm test directory");
        let config_path = temp.path().join("bmc-config.json");
        let (config_handle, _) =
            ConfigHandle::init(config_path.clone(), 50, 50, 50, 50, product).await;
        let config_handle = Arc::new(RwLock::new(config_handle));
        let (timezone_sender, timezone_receiver) = tokio::sync::watch::channel(Timezone::default());
        // Without a path of its own the scheduler writes /etc/crontabs/root,
        // which a test process may not open.
        let crontab = temp.path().join("crontabs").join("root");
        if matches!(crontab_setup, CrontabSetup::Unwritable) {
            std::fs::write(
                crontab
                    .parent()
                    .expect("BUG: the crontab path must have a parent"),
                "",
            )
            .expect("BUG: writing the crontab directory stand-in must succeed in tests");
        }
        let scheduler = JobScheduler::init(timezone_receiver.clone(), Some(crontab)).await;

        Self {
            temp,
            timezone_sender,
            config_path,
            config_handle,
            scheduler,
            bus: AlarmBus::new(),
            timezone_receiver,
        }
    }

    async fn init_backend(&self, alarm_supported: bool) -> AlarmBackend {
        AlarmBackend::init(
            alarm_supported,
            self.config_handle.clone(),
            self.scheduler.clone(),
            SoundController::new(self.config_handle.clone(), self.temp.path().join("sounds")),
            self.bus.clone(),
            self.timezone_receiver.clone(),
        )
        .await
    }

    async fn supported_controller(&self) -> AlarmController {
        self.init_backend(true)
            .await
            .controller()
            .expect("BUG: supported alarm fixture must initialize a controller")
    }
}

pub(crate) async fn supported_alarm_controller(
    product: bmc_platform::Product,
) -> (
    tempfile::TempDir,
    tokio::sync::watch::Sender<Timezone>,
    AlarmController,
) {
    let fixture = AlarmFixture::new(product).await;
    let controller = fixture.supported_controller().await;

    (fixture.temp, fixture.timezone_sender, controller)
}
