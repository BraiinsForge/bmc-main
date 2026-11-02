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

use super::{DamageHistory, EXPORT_BUFFERS, FrameRepaint, TargetContents, damage_rects};
use crate::interaction::Rect;
use crate::partition::static_layer::{AssetGeneration, FrameTree, LayerCommit, LayerPlan};
use crate::tree::StaticLayer;

/// Only a blit under some regions leaves the rest of the target alone;
/// a whole-layer blit, a capture or a miss repaints all of it, whatever damage it was handed.
#[test]
fn only_a_partial_blit_reports_a_damage_repaint() {
    let region = Rect::new(1.0, 2.0, 3.0, 4.0);
    let partial = FrameRepaint::of_walk(StaticLayer::Blitted, vec![region]);
    assert!(
        matches!(&partial, FrameRepaint::Damage { blitted } if blitted.len() == 1),
        "got {partial:?}"
    );
    let whole_blit = FrameRepaint::of_walk(StaticLayer::Blitted, Vec::new());
    assert!(
        matches!(whole_blit, FrameRepaint::Full),
        "got {whole_blit:?}"
    );
    for whole in [
        StaticLayer::Captured { empty: false },
        StaticLayer::Missed,
        StaticLayer::Unused,
    ] {
        let repaint = FrameRepaint::of_walk(whole, vec![region]);
        assert!(matches!(repaint, FrameRepaint::Full), "got {repaint:?}");
    }
}

#[test]
fn a_damage_repaint_covers_only_its_blitted_rects() {
    let blitted = vec![
        Rect::new(0.0, 0.0, 10.0, 4.0),
        Rect::new(20.0, 0.0, 5.0, 5.0),
    ];
    assert!((FrameRepaint::Damage { blitted }.area(100.0, 100.0) - 65.0).abs() < f32::EPSILON);
    assert!((FrameRepaint::Full.area(100.0, 50.0) - 5_000.0).abs() < f32::EPSILON);
}

/// A history carrying damage from two walks, as a settled tree has.
fn settled() -> DamageHistory {
    DamageHistory::settled(&[Rect::new(0.0, 0.0, 40.0, 40.0)])
}

/// Asked twice a frame, the damage is worked out once,
/// but a walk that moves the rects must not be answered from the last frame.
#[test]
fn frame_damage_follows_the_rects_it_comes_from() {
    let mut history = settled();
    assert_eq!(
        history.frame_damage(1280.0, 480.0),
        history.frame_damage(1280.0, 480.0)
    );
    let moved = Rect::new(600.0, 200.0, 40.0, 40.0);
    for _ in 0..EXPORT_BUFFERS {
        history.walked(
            LayerCommit::Reused,
            false,
            StaticLayer::Blitted,
            &[moved],
            Vec::new(),
        );
    }
    assert_eq!(history.frame_damage(1280.0, 480.0), [moved]);
}

#[test]
fn a_settled_tree_scissors_to_its_damage() {
    assert!(!settled().frame_damage(1280.0, 480.0).is_empty());
}

/// The pair spans the buffer rotation: this frame paints into the buffer the
/// frame *before* last drew, so covering only the newest walk leaves what
/// that one moved unrepainted — a trail behind anything moving.
#[test]
fn the_damage_span_covers_both_walks() {
    let newest = Rect::new(0.0, 0.0, 20.0, 20.0);
    let previous = Rect::new(60.0, 60.0, 20.0, 20.0);

    let damage = damage_rects(&[vec![newest], vec![previous]], 200.0, 200.0);

    for stale in [newest, previous] {
        assert!(
            damage.iter().any(|d| d.x <= stale.x
                && d.y <= stale.y
                && d.right() >= stale.right()
                && d.bottom() >= stale.bottom()),
            "{stale:?} is left out of the damage span {damage:?}"
        );
    }
}

/// Fresh buffers hold nothing, while the damage history survives describing
/// a target that no longer exists. Scissoring to it leaves the static half black.
#[test]
fn every_buffer_repaints_in_full_once_all_are_owed() {
    let mut history = settled();
    history.owe_every_buffer();

    for buffer in 0..EXPORT_BUFFERS {
        assert!(
            history.frame_damage(1280.0, 480.0).is_empty(),
            "buffer {buffer} has never been painted, so nothing may be preserved"
        );
        history.painted_in_full();
    }

    assert!(
        !history.frame_damage(1280.0, 480.0).is_empty(),
        "every buffer holds a full frame now, so damage tracking resumes"
    );
}

#[test]
fn painting_in_full_while_nothing_is_owed_stays_settled() {
    let mut history = settled();
    history.painted_in_full();
    assert_eq!(history.owed_full_repaints(), 0);
}

/// What the commit and the frame's freshness say about the static half,
/// as the buffers a walk leaves owed a full repaint.
#[test]
fn a_walk_owes_the_other_buffers_only_when_its_static_half_may_have_changed() {
    for (committed, fresh, owed) in [
        (LayerCommit::Reused, true, 0),
        (LayerCommit::Reused, false, 0),
        (LayerCommit::Missed, true, 0),
        (LayerCommit::Missed, false, 0),
        (LayerCommit::Rewritten, true, EXPORT_BUFFERS - 1),
        (LayerCommit::Rewritten, false, EXPORT_BUFFERS - 1),
        (LayerCommit::Unlayered, true, EXPORT_BUFFERS - 1),
        (LayerCommit::Unlayered, false, 0),
    ] {
        let mut history = settled();
        history.walked(committed, fresh, StaticLayer::Unused, &[], Vec::new());
        assert_eq!(
            history.owed_full_repaints(),
            owed,
            "{committed:?}, fresh: {fresh}"
        );
    }
}

#[test]
fn only_a_replay_over_the_layer_with_damage_keeps_the_target() {
    let history = settled();
    assert_eq!(
        history.target_for(true, 1280.0, 480.0),
        TargetContents::Preserved
    );
    assert_eq!(
        history.target_for(false, 1280.0, 480.0),
        TargetContents::Cleared
    );
    assert_eq!(
        DamageHistory::default().target_for(true, 1280.0, 480.0),
        TargetContents::Cleared,
        "nothing moved, so there is nothing to scissor to"
    );
}

#[test]
fn a_cleared_target_gets_no_damage() {
    assert!(
        settled()
            .damage_for(TargetContents::Cleared, 1280.0, 480.0)
            .is_empty()
    );
}

#[test]
fn a_walk_reports_a_damage_repaint_only_for_a_blit_under_its_damage() {
    let region = Rect::new(1.0, 2.0, 3.0, 4.0);
    let repaint = settled().walked(
        LayerCommit::Reused,
        false,
        StaticLayer::Blitted,
        &[region],
        vec![region],
    );
    assert!(
        matches!(&repaint, FrameRepaint::Damage { blitted } if blitted == &[region]),
        "got {repaint:?}"
    );
}

#[test]
fn overlapping_damage_from_two_walks_merges_into_one_rect() {
    // What the clock produces: a hand's rect from each walk, a couple of
    // pixels apart, plus a separate region elsewhere on the surface.
    let recent = [
        vec![
            Rect::new(100.0, 100.0, 200.0, 200.0),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ],
        vec![Rect::new(102.0, 100.0, 200.0, 200.0)],
    ];
    let mut rects = damage_rects(&recent, 1280.0, 480.0);
    rects.sort_by(|a, b| a.x.total_cmp(&b.x));
    let corners: Vec<(f32, f32, f32, f32)> = rects.into_iter().map(Into::into).collect();
    assert_eq!(
        corners,
        [(0.0, 0.0, 10.0, 10.0), (100.0, 100.0, 202.0, 200.0)],
        "the two hand rects merge; the disjoint one is left alone"
    );
}

#[test]
fn rects_overlapping_at_a_corner_stay_apart() {
    // A 10x10 overlap, under a bounding box twice the pair's area.
    let upper_left = Rect::new(400.0, 40.0, 210.0, 210.0);
    let lower_right = Rect::new(600.0, 240.0, 200.0, 200.0);
    let recent = [vec![upper_left, lower_right], Vec::new()];
    let mut rects = damage_rects(&recent, 1280.0, 480.0);
    rects.sort_by(|a, b| a.x.total_cmp(&b.x));
    let corners: Vec<(f32, f32, f32, f32)> = rects.into_iter().map(Into::into).collect();
    assert_eq!(corners, [upper_left.into(), lower_right.into()]);
}

#[test]
fn the_cap_is_measured_on_the_merged_area_not_the_double_count() {
    // Two walks of the same 35%-of-surface region: 70% counted separately,
    // 35% once merged — the difference decides whether tracking survives.
    let region = Rect::new(0.0, 0.0, 1280.0 * 0.35, 480.0);
    let recent = [vec![region], vec![region]];
    let merged: Vec<(f32, f32, f32, f32)> = damage_rects(&recent, 1280.0, 480.0)
        .into_iter()
        .map(Into::into)
        .collect();
    assert_eq!(
        merged,
        [region.into()],
        "a tree under the cap must not be pushed over it by its own history"
    );
}

/// A key for a layer the [`layered`] tree plans with.
fn key() -> crate::partition::static_layer::LayerKey {
    crate::partition::static_layer::LayerKey {
        size: (1280, 480),
        static_hash: crate::partition::StaticHash(1),
        layout: crate::partition::LayoutKey(0),
        assets: AssetGeneration::Unchanging,
    }
}

fn layered() -> super::LayeredTree {
    use crate::partition::static_layer::{MissPolicy, StaticLayerCache};
    use crate::renderer::{LayerOwner, OverlayId};
    let mut layered = super::LayeredTree::new(StaticLayerCache::new(
        LayerOwner::Overlay(OverlayId::next()),
        MissPolicy::Release,
    ));
    *layered.damage_mut() = settled();
    layered
}

/// A walk that failed may have left its target half painted,
/// so no buffer holds what the history describes.
#[test]
fn a_failed_walk_owes_every_buffer() {
    let mut layered = layered();
    layered.fail();
    assert_eq!(layered.damage().owed_full_repaints(), EXPORT_BUFFERS);
}

/// A miss stops the layer, and a frame painting without one repaints in full.
#[test]
fn a_stopped_layer_keeps_no_target() {
    let mut renderer = crate::renderer::test_support::ShapingRecorder::default();
    let mut layered = layered();
    assert_eq!(
        layered.target_for(true, 1280.0, 480.0),
        TargetContents::Preserved,
        "the fixture must be one a layered replay would keep"
    );
    layered.static_layer_mut().commit(
        &mut renderer,
        LayerPlan::Capture(key()),
        StaticLayer::Missed,
        AssetGeneration::Unchanging,
    );
    assert_eq!(
        layered.target_for(true, 1280.0, 480.0),
        TargetContents::Cleared
    );
}

/// The damage a frame scissors to and how it repainted come from the one plan.
#[test]
fn a_finished_frame_reports_the_damage_it_was_planned_with() {
    let mut renderer =
        crate::renderer::test_support::ShapingRecorder::new(1280.0, 480.0).with_static_layers();
    let mut layered = layered();
    let moved = Rect::new(0.0, 0.0, 40.0, 40.0);
    let tree = || FrameTree {
        key: Some(key()),
        has_static_content: true,
        scrolling: false,
        fresh: false,
    };
    // A capture, then a full repaint that catches the other buffer up on it.
    for (target, walked) in [
        (
            TargetContents::Cleared,
            StaticLayer::Captured { empty: false },
        ),
        (TargetContents::Cleared, StaticLayer::Blitted),
    ] {
        let frame = layered.begin(&mut renderer, tree(), target, 1280.0, 480.0);
        let _ = layered.finish(
            &mut renderer,
            frame,
            walked,
            &[moved],
            AssetGeneration::Unchanging,
        );
    }
    let frame = layered.begin(
        &mut renderer,
        tree(),
        TargetContents::Preserved,
        1280.0,
        480.0,
    );
    assert_eq!(frame.damage(), [moved]);
    let repaint = layered.finish(
        &mut renderer,
        frame,
        StaticLayer::Blitted,
        &[moved],
        AssetGeneration::Unchanging,
    );
    assert!(
        matches!(&repaint, FrameRepaint::Damage { blitted } if blitted == &[moved]),
        "got {repaint:?}"
    );
}
