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

use std::fmt::Debug;

/// Trait for controlling display backlight hardware.
pub trait DisplayBacklightDriver: Sync + Send + Clone + Debug + 'static {
    fn init(&mut self) -> anyhow::Result<()>;

    fn change_state(&self, enabled: bool) -> anyhow::Result<()>;

    fn state(&self) -> anyhow::Result<bool>;

    fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    fn brightness(&self) -> anyhow::Result<u8>;

    fn max_brightness(&self) -> u8;

    fn set_brightness(&self, value: u8) -> anyhow::Result<()>;

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::integer_division)]
    fn pct_to_brightness(&self, percent: u8) -> u8 {
        ((u16::from(percent) * u16::from(self.max_brightness())) / 100) as u8
    }

    fn set_brightness_pct(&self, percent: u8) -> anyhow::Result<()> {
        self.set_brightness(self.pct_to_brightness(percent))
    }

    /// Whether the panel is showing the user anything.
    ///
    /// Powered *and* non-zero brightness. Both halves matter: auto-off zeroes
    /// brightness before it cuts power and restores it after power returns, so
    /// each sequence has a window where one attribute alone reports a black
    /// panel as visible.
    ///
    /// The one definition every consumer delegates to — a second copy of the
    /// predicate is how the blank and wake paths end up disagreeing about
    /// whether the panel is dark.
    fn is_visible(&self) -> anyhow::Result<bool> {
        Ok(self.state()? && self.brightness()? > 0)
    }
}

/// Read-only view of whether the panel is showing the user anything.
///
/// Exists so consumers that merely *react* to screen power — the compositor
/// swallowing the touch that wakes a dark panel, for one — can ask the driver
/// that owns the state instead of mirroring it. A mirrored copy updates on its
/// own schedule and drifts; this cannot.
pub trait ScreenVisibility: Send + Sync + Debug + 'static {
    /// Errors are returned rather than folded into `false`: a failed read says
    /// nothing about the panel, and callers that treat "unknown" as "dark"
    /// swallow input on a screen the user can see.
    fn is_visible(&self) -> anyhow::Result<bool>;
}

/// [`ScreenVisibility`] backed by a [`DisplayBacklightDriver`].
#[derive(Debug, Clone)]
pub struct BacklightVisibility<T: DisplayBacklightDriver> {
    driver: T,
}

impl<T: DisplayBacklightDriver> BacklightVisibility<T> {
    pub const fn new(driver: T) -> Self {
        Self { driver }
    }
}

impl<T: DisplayBacklightDriver> ScreenVisibility for BacklightVisibility<T> {
    fn is_visible(&self) -> anyhow::Result<bool> {
        self.driver.is_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::{BacklightVisibility, DisplayBacklightDriver, ScreenVisibility};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    /// Driver whose power, brightness and readability are all scriptable, so a
    /// test can pose the partially-blanked states auto-off passes through.
    #[derive(Debug, Clone, Default)]
    struct ScriptedDriver {
        powered: Arc<AtomicBool>,
        brightness: Arc<AtomicU8>,
        readable: Arc<AtomicBool>,
    }

    impl ScriptedDriver {
        fn new(powered: bool, brightness: u8) -> Self {
            Self {
                powered: Arc::new(AtomicBool::new(powered)),
                brightness: Arc::new(AtomicU8::new(brightness)),
                readable: Arc::new(AtomicBool::new(true)),
            }
        }

        fn fail_reads(&self) {
            self.readable.store(false, Ordering::Relaxed);
        }
    }

    impl DisplayBacklightDriver for ScriptedDriver {
        fn init(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn change_state(&self, enabled: bool) -> anyhow::Result<()> {
            self.powered.store(enabled, Ordering::Relaxed);
            Ok(())
        }

        fn state(&self) -> anyhow::Result<bool> {
            if self.readable.load(Ordering::Relaxed) {
                Ok(self.powered.load(Ordering::Relaxed))
            } else {
                anyhow::bail!("simulated power read failure")
            }
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

    #[test]
    fn panel_is_visible_only_when_powered_and_lit() {
        assert!(
            ScriptedDriver::new(true, 50)
                .is_visible()
                .expect("BUG: a readable driver must report visibility"),
            "powered with non-zero brightness is the only visible state"
        );
        assert!(
            !ScriptedDriver::new(false, 50)
                .is_visible()
                .expect("BUG: a readable driver must report visibility"),
            "an unpowered panel shows nothing whatever its brightness reads"
        );
        assert!(
            !ScriptedDriver::new(true, 0)
                .is_visible()
                .expect("BUG: a readable driver must report visibility"),
            "brightness zero is the state a power-only predicate misreads as lit"
        );
    }

    #[test]
    fn unreadable_driver_returns_the_error_rather_than_guessing() {
        let driver = ScriptedDriver::new(true, 50);
        driver.fail_reads();

        assert!(
            driver.is_visible().is_err(),
            "a failed read says nothing about the panel and must not fold to false"
        );
    }

    #[test]
    fn visibility_port_delegates_to_the_driver_predicate() {
        // The whole point of the shared default: the port the compositor holds
        // and the driver the auto-off loop reaches cannot disagree.
        let driver = ScriptedDriver::new(true, 0);
        let port = BacklightVisibility::new(driver.clone());

        assert_eq!(
            port.is_visible()
                .expect("BUG: a readable driver must report visibility"),
            driver
                .is_visible()
                .expect("BUG: a readable driver must report visibility"),
            "the port must not carry a second copy of the predicate"
        );
    }
}
