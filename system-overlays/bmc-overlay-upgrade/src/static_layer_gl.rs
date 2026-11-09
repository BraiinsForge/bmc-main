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

use bmc_system_overlay::gl_check::{
    CAPTURED, GlRun, REUSED, assert_layered_run_matches_full_paint,
    assert_moving_damage_matches_full_paint,
};
use bmc_system_overlay::{DownloadProgress, FrameKind, UpgradeKind, UpgradePhase};

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

/// One upgrade surface, its frames stamped by the run's clock.
struct Card {
    state: UpgradeRenderState,
}

impl Card {
    fn new(run: &GlRun<'_>) -> Self {
        Self {
            state: UpgradeRenderState::new(run.now()),
        }
    }

    fn paint(&mut self, run: &mut GlRun<'_>, view: &UpgradeView, kind: FrameKind) {
        let surface = Surface {
            width: SIZE.0,
            height: SIZE.1,
            tier: SurfaceTier::fullscreen_for_width(SIZE.0),
        };
        let state = &mut self.state;
        run.paint(
            kind,
            state,
            |state| Some(&mut state.tree),
            |state, renderer, frame| {
                render_upgrade(
                    renderer,
                    surface,
                    state,
                    view,
                    &bmc_intl::formatter(bmc_intl::Language::default()),
                    frame,
                )
            },
        );
    }
}

#[test]
fn a_progress_run_paints_as_a_full_paint_does() {
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let mut card = Card::new(run);
        let no_total = downloading(None, 0);
        card.paint(run, &no_total, FrameKind::Fresh);
        for _ in 0..9 {
            card.paint(run, &no_total, FrameKind::AnimationOnly);
        }
        for step in 0..=20 {
            let view = downloading(Some(1_000_000), step * 50_000);
            card.paint(run, &view, FrameKind::Fresh);
            card.paint(run, &view, FrameKind::Fresh);
        }
    });
    let Some(outcomes) = outcomes else { return };
    assert_eq!(
        outcomes[..10],
        [[CAPTURED].as_slice(), &[REUSED; 9]].concat(),
        "the card without a total captures once and reuses it while the bar moves"
    );
    let (first_step, later_steps) = outcomes[10..].split_at(2);
    assert_eq!(
        first_step,
        [CAPTURED, REUSED],
        "the first progress step draws the determinate card's static half anew"
    );
    for (step, pair) in later_steps.chunks(2).enumerate() {
        assert_eq!(
            pair,
            [REUSED, REUSED],
            "progress step {} keeps the static half, as only the progress moved",
            step + 1
        );
    }
}

#[test]
fn two_cards_of_one_kind_keep_their_own_layers() {
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let (mut first, mut second) = (Card::new(run), Card::new(run));
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
    assert_eq!(outcomes[..2], [CAPTURED, CAPTURED]);
    assert!(
        outcomes[2..].iter().all(|outcome| *outcome == REUSED),
        "a shared layer would be captured over by the other card every frame: {outcomes:?}"
    );
}

/// The card has no total, as a determinate card's text is unbounded
/// and every replay of it would repaint the whole surface.
#[test]
fn moving_damage_on_a_progress_card_paints_as_a_full_paint_does() {
    let surface = Surface {
        width: SIZE.0,
        height: SIZE.1,
        tier: SurfaceTier::fullscreen_for_width(SIZE.0),
    };
    let view = downloading(None, 400);
    let intl = bmc_intl::formatter(bmc_intl::Language::default());
    let _ = assert_moving_damage_matches_full_paint(SIZE, crate::icons::register_icons, |icons| {
        crate::ui::build_upgrade_tree(&view, surface, *icons, &intl)
    });
}
