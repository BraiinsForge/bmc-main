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

use crate::backlight::DisplayBacklightDriver;
use crate::compositor::Compositor;
use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_led::led_driver::LedDriver;
use bmc_upgrade::firmware::FirmwareIndex;
use bmc_upgrade::packages::PackageBackend;
use std::sync::Arc;
use tokio::sync::Mutex;

#[expect(
    clippy::too_many_arguments,
    reason = "the entry point takes one argument per injected subsystem"
)]
pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    backlight_driver: Arc<Mutex<T>>,
    led_driver: LedDriver,
    firmware_index: U,
    package_backend: Arc<dyn PackageBackend>,
    buttons: Arc<Box<dyn bmc_button::Buttons + Send + Sync>>,
    compositor: Arc<dyn Compositor>,
    wayland_display: Option<String>,
) -> Result<()> {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let app = App::init(
        config,
        manager,
        session_manager,
        backlight_driver,
        led_driver,
        firmware_index,
        package_backend,
        buttons,
        compositor,
        wayland_display,
    )
    .await?;
    app.run().await
}
