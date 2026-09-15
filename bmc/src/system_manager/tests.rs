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

mod auto_off_decision;
mod auto_off_loop;
mod local_time;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use super::*;

#[derive(Debug, Clone)]
struct DummyBacklightDriver;

impl DisplayBacklightDriver for DummyBacklightDriver {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }
    fn state(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
    fn brightness(&self) -> anyhow::Result<u8> {
        Ok(100)
    }
    fn max_brightness(&self) -> u8 {
        255
    }
    fn set_brightness(&self, _value: u8) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Driver whose power and brightness are scriptable so a test can pose the
/// half-blanked panel a failed `turn_off` leaves behind.
#[derive(Debug, Clone)]
struct ScriptedBacklightDriver {
    powered: Arc<AtomicBool>,
    brightness: Arc<AtomicU8>,
}

impl ScriptedBacklightDriver {
    fn new(powered: bool, brightness: u8) -> Self {
        Self {
            powered: Arc::new(AtomicBool::new(powered)),
            brightness: Arc::new(AtomicU8::new(brightness)),
        }
    }
}

impl DisplayBacklightDriver for ScriptedBacklightDriver {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn change_state(&self, enabled: bool) -> anyhow::Result<()> {
        self.powered.store(enabled, Ordering::Relaxed);
        Ok(())
    }
    fn state(&self) -> anyhow::Result<bool> {
        Ok(self.powered.load(Ordering::Relaxed))
    }
    fn brightness(&self) -> anyhow::Result<u8> {
        Ok(self.brightness.load(Ordering::Relaxed))
    }
    fn max_brightness(&self) -> u8 {
        255
    }
    fn set_brightness(&self, value: u8) -> anyhow::Result<()> {
        self.brightness.store(value, Ordering::Relaxed);
        Ok(())
    }
}

async fn scripted_controller(
    driver: ScriptedBacklightDriver,
) -> (
    tempfile::TempDir,
    DisplayBacklightController<ScriptedBacklightDriver>,
) {
    let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
    let (handle, _extracted_accounts) = ConfigHandle::init(
        tmp.path().join("bmc-config.json"),
        50,
        50,
        50,
        50,
        bmc_platform::Product::Bmc100,
    )
    .await;
    let controller = DisplayBacklightController::new(
        Arc::new(RwLock::new(handle)),
        Arc::new(Mutex::new(driver)),
    );
    (tmp, controller)
}

#[tokio::test]
async fn a_panel_dimmed_to_zero_reads_as_dark_though_power_stayed_on() {
    // The state a failed `turn_off` strands: brightness zeroed, power pin
    // still high. The power-only predicate called this lit, so the activity
    // arm skipped the wake and brightness was never restored — a black
    // screen that ate every touch.
    let (_tmp, controller) = scripted_controller(ScriptedBacklightDriver::new(true, 0)).await;

    assert!(
        SystemManager::<ScriptedBacklightDriver>::is_screen_dark(&controller).await,
        "a panel dimmed to zero is dark whatever the power pin says"
    );
}

#[tokio::test]
async fn a_lit_panel_is_not_dark() {
    let (_tmp, controller) = scripted_controller(ScriptedBacklightDriver::new(true, 50)).await;

    assert!(
        !SystemManager::<ScriptedBacklightDriver>::is_screen_dark(&controller).await,
        "a powered, lit panel must never be woken or announced as blanked"
    );
}

#[tokio::test]
async fn an_unpowered_panel_is_dark() {
    let (_tmp, controller) = scripted_controller(ScriptedBacklightDriver::new(false, 50)).await;

    assert!(
        SystemManager::<ScriptedBacklightDriver>::is_screen_dark(&controller).await,
        "a fully blanked panel must read as dark so the wake path fires"
    );
}
