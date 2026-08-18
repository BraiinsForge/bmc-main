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

//! Upgrade-progress overlays sharing one state-driven renderer.

mod icons;
mod ui;

use std::time::{Duration, Instant};

use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    Anchor, DownloadProgress, InputRegion, Layer, LayerConfig, SystemOverlay, TickOutcome, TreeUi,
    UpgradeKind, UpgradePhase, UpgradeSnapshot, UpgradeState,
};

use crate::icons::UpgradeIcons;

// Package realization can run for minutes under CPU and flash load, so keep
// the indeterminate bar at 10 fps.
const ANIMATION_FRAME: Duration = Duration::from_millis(100);

/// Compact package surface size, shared by the runtime and the gallery scene.
pub const PACKAGE_SURFACE_SIZE: (u32, u32) = (384, 192);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeView {
    Running {
        kind: UpgradeKind,
        phase: Option<UpgradePhase>,
        progress: Option<DownloadProgress>,
    },
    /// Only ever built for `UpgradeKind::Packages` — see `OverlayState::shows`.
    Succeeded {
        kind: UpgradeKind,
    },
    Failed {
        kind: UpgradeKind,
    },
}

impl UpgradeView {
    #[must_use]
    pub fn kind(self) -> UpgradeKind {
        match self {
            Self::Running { kind, .. } | Self::Succeeded { kind } | Self::Failed { kind } => kind,
        }
    }
}

impl From<UpgradeSnapshot> for UpgradeView {
    fn from(snapshot: UpgradeSnapshot) -> Self {
        match snapshot.state {
            UpgradeState::Running { phase, progress } => Self::Running {
                kind: snapshot.kind,
                phase,
                progress,
            },
            UpgradeState::Succeeded { .. } => Self::Succeeded {
                kind: snapshot.kind,
            },
            UpgradeState::Failed { .. } => Self::Failed {
                kind: snapshot.kind,
            },
        }
    }
}

#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct UpgradeRenderState {
    tree: TreeUi,
    icons: Option<UpgradeIcons>,
    last_render: Instant,
}

impl UpgradeRenderState {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            tree: TreeUi::new(),
            icons: None,
            last_render: now,
        }
    }
}

/// Render an injected snapshot view with retained layout and registered icons.
pub fn render_upgrade(
    renderer: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut UpgradeRenderState,
    view: &UpgradeView,
    now: Instant,
) {
    let icons = *state
        .icons
        .get_or_insert_with(|| icons::register_icons(renderer));
    let delta_ms = u32::try_from(now.saturating_duration_since(state.last_render).as_millis())
        .unwrap_or(u32::MAX);
    state.last_render = now;
    let tree = ui::build_upgrade_tree(view, size, icons);
    if let Err(err) = state.tree.render(&tree, size, delta_ms, renderer) {
        tracing::error!("upgrade tree render failed: {err}");
    }
}

struct OverlayState {
    kind: UpgradeKind,
    view: Option<UpgradeView>,
    terminal_deadline: Option<Instant>,
    dirty: bool,
    render_state: UpgradeRenderState,
}

impl OverlayState {
    fn new(kind: UpgradeKind) -> Self {
        Self {
            kind,
            view: None,
            terminal_deadline: None,
            dirty: false,
            render_state: UpgradeRenderState::new(Instant::now()),
        }
    }

    fn clear(&mut self) {
        self.view = None;
        self.terminal_deadline = None;
        self.dirty = false;
    }

    /// Whether this surface presents `snapshot`. The other kind belongs
    /// to the sibling overlay, and a firmware success to the device-info overlay,
    /// which shows it and continues into the boot connect flow from there —
    /// showing it here too would stack two screens over one boot.
    fn shows(&self, snapshot: UpgradeSnapshot) -> bool {
        snapshot.kind == self.kind
            && !(self.kind == UpgradeKind::Firmware
                && matches!(snapshot.state, UpgradeState::Succeeded { .. }))
    }

    fn receive(&mut self, snapshot: UpgradeSnapshot, now: Instant) {
        if !self.shows(snapshot) {
            self.clear();
            return;
        }
        let deadline = match snapshot.state {
            UpgradeState::Succeeded { remaining } | UpgradeState::Failed { remaining } => {
                now.checked_add(remaining)
            }
            UpgradeState::Running { .. } => None,
        };
        let view = UpgradeView::from(snapshot);
        if self.view != Some(view) {
            self.dirty = true;
        }
        self.view = Some(view);
        self.terminal_deadline = deadline;
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if self
            .terminal_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.clear();
        }
        let visible = self.view.is_some();
        let animating = self.view.is_some_and(ui::has_active_bar);
        TickOutcome {
            visible,
            wants_render: visible && (std::mem::take(&mut self.dirty) || animating),
            next_wake: if animating {
                Some(now + ANIMATION_FRAME)
            } else {
                self.terminal_deadline
            },
        }
    }

    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32)) {
        if let Some(view) = self.view {
            render_upgrade(
                renderer,
                size,
                &mut self.render_state,
                &view,
                Instant::now(),
            );
        }
    }
}

#[expect(
    missing_debug_implementations,
    reason = "retained TreeUi render state is not Debug"
)]
pub struct UpgradeOverlay {
    state: OverlayState,
}

impl UpgradeOverlay {
    #[must_use]
    pub fn firmware() -> Self {
        Self {
            state: OverlayState::new(UpgradeKind::Firmware),
        }
    }

    #[must_use]
    pub fn packages() -> Self {
        Self {
            state: OverlayState::new(UpgradeKind::Packages),
        }
    }
}

impl SystemOverlay for UpgradeOverlay {
    fn layer_config(&self) -> LayerConfig {
        match self.state.kind {
            UpgradeKind::Firmware => LayerConfig::fullscreen("bmc-overlay-upgrade-firmware"),
            UpgradeKind::Packages => LayerConfig {
                layer: Layer::Bottom,
                anchor: Anchor::Bottom | Anchor::Right,
                size: PACKAGE_SURFACE_SIZE,
                margin_top: 0,
                margin_right: 0,
                margin_bottom: 0,
                margin_left: 0,
                exclusive_zone: 0,
                namespace: "bmc-overlay-upgrade-packages".to_owned(),
                input: InputRegion::None,
            },
            kind => unreachable!("BUG: upgrade overlay constructed for unsupported kind {kind:?}"),
        }
    }

    fn uses_upgrade(&self) -> bool {
        true
    }

    fn on_upgrade_state(&mut self, snapshot: UpgradeSnapshot) {
        self.state.receive(snapshot, Instant::now());
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        self.state.tick(now)
    }

    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32)) {
        self.state.render(renderer, size);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn running(kind: UpgradeKind) -> UpgradeSnapshot {
        UpgradeSnapshot {
            kind,
            state: UpgradeState::Running {
                phase: None,
                progress: None,
            },
        }
    }

    #[test]
    fn overlays_ignore_the_other_upgrade_kind() {
        let now = Instant::now();
        let mut firmware = OverlayState::new(UpgradeKind::Firmware);
        firmware.receive(running(UpgradeKind::Packages), now);
        assert!(!firmware.tick(now).visible);
    }

    #[test]
    fn a_new_opposite_kind_clears_the_previous_terminal_view() {
        let now = Instant::now();
        let mut package = OverlayState::new(UpgradeKind::Packages);
        package.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Packages,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_secs(1),
                },
            },
            now,
        );
        assert!(package.tick(now).visible);

        package.receive(running(UpgradeKind::Firmware), now);
        assert!(!package.tick(now).visible);
    }

    #[test]
    fn equal_snapshots_do_not_queue_another_render() {
        let now = Instant::now();
        let mut state = OverlayState::new(UpgradeKind::Packages);
        let snapshot = running(UpgradeKind::Packages);
        state.receive(snapshot, now);
        assert!(state.tick(now).wants_render);
        state.receive(snapshot, now);
        assert!(!state.tick(now).wants_render);
    }

    #[test]
    fn terminal_countdown_updates_do_not_redraw_unchanged_content() {
        let now = Instant::now();
        let mut state = OverlayState::new(UpgradeKind::Packages);
        state.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Packages,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_secs(5),
                },
            },
            now,
        );
        assert!(state.tick(now).wants_render);

        state.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Packages,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_secs(4),
                },
            },
            now,
        );

        assert!(!state.tick(now).wants_render);
        assert_eq!(state.terminal_deadline, Some(now + Duration::from_secs(4)));
    }

    #[test]
    fn active_progress_schedules_animation_frames() {
        let now = Instant::now();
        let mut state = OverlayState::new(UpgradeKind::Packages);
        state.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Packages,
                state: UpgradeState::Running {
                    phase: Some(UpgradePhase::PackageVerifying),
                    progress: None,
                },
            },
            now,
        );

        let outcome = state.tick(now);
        assert!(outcome.wants_render);
        assert_eq!(outcome.next_wake, Some(now + Duration::from_millis(100)));
    }

    #[test]
    fn terminal_deadline_hides_the_overlay() {
        let now = Instant::now();
        let mut state = OverlayState::new(UpgradeKind::Packages);
        state.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Packages,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_millis(10),
                },
            },
            now,
        );
        assert!(state.tick(now).visible);
        assert!(!state.tick(now + Duration::from_millis(10)).visible);
    }

    #[test]
    fn failure_replaces_running_progress_immediately() {
        let now = Instant::now();
        let mut state = OverlayState::new(UpgradeKind::Firmware);
        state.receive(running(UpgradeKind::Firmware), now);
        assert!(state.tick(now).wants_render);

        state.receive(
            UpgradeSnapshot {
                kind: UpgradeKind::Firmware,
                state: UpgradeState::Failed {
                    remaining: Duration::from_secs(5),
                },
            },
            now,
        );

        assert_eq!(
            state.view,
            Some(UpgradeView::Failed {
                kind: UpgradeKind::Firmware
            })
        );
        assert!(state.tick(now).wants_render);
    }

    #[test]
    fn a_firmware_success_maps_nothing_here() {
        let now = Instant::now();
        let mut overlay = UpgradeOverlay::firmware();
        overlay.on_upgrade_state(running(UpgradeKind::Firmware));
        assert!(overlay.tick(now).visible);

        overlay.on_upgrade_state(UpgradeSnapshot {
            kind: UpgradeKind::Firmware,
            state: UpgradeState::Succeeded {
                remaining: Duration::from_secs(10),
            },
        });
        assert!(
            !overlay.tick(now).visible,
            "the device-info overlay owns the post-upgrade success screen"
        );
    }

    #[test]
    fn a_firmware_failure_still_stays_up_for_its_dwell() {
        let now = Instant::now();
        let mut overlay = UpgradeOverlay::firmware();
        overlay.on_upgrade_state(UpgradeSnapshot {
            kind: UpgradeKind::Firmware,
            state: UpgradeState::Failed {
                remaining: Duration::from_secs(10),
            },
        });
        assert!(overlay.tick(now).visible);
    }

    #[test]
    fn layer_configs_keep_firmware_modal_and_packages_passive() {
        let firmware = UpgradeOverlay::firmware().layer_config();
        assert_eq!(firmware.layer, Layer::Top);
        assert_eq!(
            firmware.anchor,
            Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right
        );
        assert_eq!(firmware.size, (0, 0));
        assert_eq!(
            (
                firmware.margin_top,
                firmware.margin_right,
                firmware.margin_bottom,
                firmware.margin_left,
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(firmware.exclusive_zone, 0);
        assert_eq!(firmware.namespace, "bmc-overlay-upgrade-firmware");
        assert_eq!(firmware.input, InputRegion::Full);

        let packages = UpgradeOverlay::packages().layer_config();
        assert_eq!(packages.layer, Layer::Bottom);
        assert_eq!(packages.anchor, Anchor::Bottom | Anchor::Right);
        assert_eq!(packages.size, PACKAGE_SURFACE_SIZE);
        assert_eq!(
            (
                packages.margin_top,
                packages.margin_right,
                packages.margin_bottom,
                packages.margin_left,
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(packages.exclusive_zone, 0);
        assert_eq!(packages.namespace, "bmc-overlay-upgrade-packages");
        assert_eq!(packages.input, InputRegion::None);
    }
}
