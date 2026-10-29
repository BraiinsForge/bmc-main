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

use std::time::Instant;

use bmc_render::tree::StaticLayer;
use bmc_system_overlay::gl_check::{FRAME_STEP, GlRun, assert_layered_run_matches_full_paint};
use bmc_system_overlay::{DownloadProgress, FrameKind, OverlayFrame, UpgradeKind, UpgradePhase};

use super::{Surface, SurfaceTier, UpgradeRenderState, UpgradeView, render_upgrade};

const SIZE: (u32, u32) = (1_280, 480);

fn downloading(total_bytes: Option<u64>, downloaded_bytes: u64) -> UpgradeView {
    UpgradeView::Running {
        kind: UpgradeKind::Firmware,
        phase: Some(UpgradePhase::FirmwareDownloading),
        progress: Some(DownloadProgress {
            downloaded_bytes,
            total_bytes,
        }),
    }
}

/// One upgrade surface, its frames stamped a fixed step apart,
/// so both passes of a run animate alike.
struct Card {
    state: UpgradeRenderState,
    started: Instant,
    frames: u32,
}

impl Card {
    fn new() -> Self {
        let started = Instant::now();
        Self {
            state: UpgradeRenderState::new(started),
            started,
            frames: 0,
        }
    }

    fn paint(&mut self, run: &mut GlRun<'_>, view: &UpgradeView, kind: FrameKind) {
        self.frames += 1;
        let at = self.started + FRAME_STEP * self.frames;
        let surface = Surface {
            width: SIZE.0,
            height: SIZE.1,
            tier: SurfaceTier::fullscreen_for_width(SIZE.0),
        };
        let state = &mut self.state;
        run.paint(|renderer, layering| {
            render_upgrade(
                renderer,
                surface,
                state,
                view,
                &bmc_intl::formatter(bmc_intl::Language::default()),
                OverlayFrame {
                    kind,
                    layering,
                    now: at,
                },
            )
        });
    }
}

#[test]
fn a_progress_run_paints_as_a_full_paint_does() {
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let mut card = Card::new();
        let no_total = downloading(None, 0);
        card.paint(run, &no_total, FrameKind::Fresh);
        for _ in 0..9 {
            card.paint(run, &no_total, FrameKind::AnimationOnly);
        }
        run.lose_layer(&card.state.tree);
        card.paint(run, &no_total, FrameKind::AnimationOnly);
        card.paint(run, &no_total, FrameKind::AnimationOnly);
        run.hide(&mut card.state.tree);
        for step in 0..=20 {
            let view = downloading(Some(1_000_000), step * 50_000);
            card.paint(run, &view, FrameKind::Fresh);
            card.paint(run, &view, FrameKind::Fresh);
        }
    });
    let Some(outcomes) = outcomes else { return };
    assert_eq!(
        outcomes[..12],
        [
            [StaticLayer::Captured { empty: false }].as_slice(),
            &[StaticLayer::Blitted; 9],
            &[StaticLayer::Missed, StaticLayer::Unused],
        ]
        .concat(),
        "a lost layer misses once, then the card paints uncached until it is hidden"
    );
    for (step, pair) in outcomes[12..].chunks(2).enumerate() {
        assert_eq!(
            pair,
            [StaticLayer::Captured { empty: false }, StaticLayer::Blitted],
            "progress step {step} captures its new static half and reuses it"
        );
    }
}

#[test]
fn two_cards_of_one_kind_keep_their_own_layers() {
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let (mut first, mut second) = (Card::new(), Card::new());
        let (quarter, three_quarters) = (
            downloading(Some(1_000_000), 250_000),
            downloading(Some(1_000_000), 750_000),
        );
        for _ in 0..16 {
            first.paint(run, &quarter, FrameKind::Fresh);
            second.paint(run, &three_quarters, FrameKind::Fresh);
        }
    });
    let Some(outcomes) = outcomes else { return };
    assert_eq!(
        outcomes[..2],
        [
            StaticLayer::Captured { empty: false },
            StaticLayer::Captured { empty: false }
        ]
    );
    assert!(
        outcomes[2..]
            .iter()
            .all(|outcome| *outcome == StaticLayer::Blitted),
        "a shared layer would be captured over by the other card every frame: {outcomes:?}"
    );
}
