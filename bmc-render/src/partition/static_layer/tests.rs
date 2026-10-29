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

use super::{
    AssetGeneration, FrameTree, LayerCommit, LayerKey, LayerPlan, MissPolicy, StaticLayerCache,
};
use crate::partition::{LayoutKey, StaticHash};
use crate::renderer::LayerOwner;
use crate::renderer::prelude::*;
use crate::renderer::test_support::ShapingRecorder;
use crate::tree::{Node, StaticLayer};
use bmc_ui::{Color, PropsData};

fn owner() -> LayerOwner {
    LayerOwner::Widget("test".to_owned())
}
const CAPTURED: StaticLayer = StaticLayer::Captured { empty: false };

fn key(static_hash: u64) -> LayerKey {
    LayerKey {
        size: (100, 20),
        static_hash: StaticHash(static_hash),
        layout: LayoutKey(0),
        assets: AssetGeneration::Unchanging,
    }
}

fn fresh(key: LayerKey) -> FrameTree {
    FrameTree {
        key: Some(key),
        has_static_content: true,
        scrolling: false,
        fresh: true,
    }
}

fn replay(key: LayerKey) -> FrameTree {
    FrameTree {
        fresh: false,
        ..fresh(key)
    }
}

fn layers() -> ShapingRecorder {
    ShapingRecorder::new(100.0, 20.0).with_static_layers()
}

fn cache(policy: MissPolicy) -> StaticLayerCache {
    StaticLayerCache::new(owner(), policy)
}

/// Plan a frame and commit what its walk is taken to have done with the layer.
fn frame(
    cache: &mut StaticLayerCache,
    renderer: &mut ShapingRecorder,
    tree: FrameTree,
    layer: StaticLayer,
) -> LayerPlan {
    let plan = cache.plan(renderer, tree);
    cache.commit(renderer, plan, layer, AssetGeneration::Unchanging);
    plan
}

/// Capture `key` so the renderer holds a layer under the owner.
fn captured(policy: MissPolicy, renderer: &mut ShapingRecorder, key: LayerKey) -> StaticLayerCache {
    let mut cache = cache(policy);
    assert!(renderer.begin_static_layer(&owner(), 100, 20));
    frame(&mut cache, renderer, fresh(key), CAPTURED);
    cache
}

#[test]
fn a_first_frame_captures() {
    let mut cache = cache(MissPolicy::Release);
    assert_eq!(
        cache.plan(&mut layers(), fresh(key(1))),
        LayerPlan::Capture(key(1))
    );
}

#[test]
fn a_frame_keyed_like_the_capture_reuses_it() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    assert_eq!(cache.plan(&mut r, replay(key(1))), LayerPlan::Reuse(key(1)));
    assert_eq!(cache.plan(&mut r, fresh(key(1))), LayerPlan::Reuse(key(1)));
}

#[test]
fn a_frame_keyed_otherwise_captures_again() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    assert_eq!(
        cache.plan(&mut r, fresh(key(2))),
        LayerPlan::Capture(key(2))
    );
}

#[test]
fn a_scrolling_frame_captures_again() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let scrolling = FrameTree {
        scrolling: true,
        ..replay(key(1))
    };
    assert_eq!(cache.plan(&mut r, scrolling), LayerPlan::Capture(key(1)));
}

#[test]
fn a_tree_without_static_content_frees_the_layer() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let bare = FrameTree {
        has_static_content: false,
        ..fresh(key(1))
    };
    assert_eq!(cache.plan(&mut r, bare), LayerPlan::Ignore);
    assert!(r.static_layer_owners().is_empty());
    assert!(!cache.is_on());
}

#[test]
fn an_unkeyed_tree_frees_the_layer() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let unkeyed = FrameTree {
        key: None,
        ..fresh(key(1))
    };
    assert_eq!(cache.plan(&mut r, unkeyed), LayerPlan::Ignore);
    assert!(r.static_layer_owners().is_empty());
}

#[test]
fn an_empty_capture_frees_the_layer_and_its_key_goes_without() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    assert!(r.begin_static_layer(&owner(), 100, 20));
    frame(
        &mut cache,
        &mut r,
        fresh(key(1)),
        StaticLayer::Captured { empty: true },
    );
    assert!(r.static_layer_owners().is_empty());
    assert!(
        !cache.is_on(),
        "a host would preserve a target the next frame paints without a layer"
    );
    assert_eq!(cache.plan(&mut r, fresh(key(1))), LayerPlan::Ignore);
    assert_eq!(
        cache.plan(&mut r, fresh(key(2))),
        LayerPlan::Capture(key(2))
    );
}

#[test]
fn returning_to_an_empty_key_frees_the_layer_another_tree_left() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    frame(
        &mut cache,
        &mut r,
        fresh(key(1)),
        StaticLayer::Captured { empty: true },
    );
    assert!(r.begin_static_layer(&owner(), 100, 20));
    frame(&mut cache, &mut r, fresh(key(2)), CAPTURED);
    assert_eq!(cache.plan(&mut r, fresh(key(1))), LayerPlan::Ignore);
    assert!(r.static_layer_owners().is_empty());
}

#[test]
fn a_miss_frees_the_layer() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    frame(&mut cache, &mut r, replay(key(1)), StaticLayer::Missed);
    assert!(r.static_layer_owners().is_empty());
}

#[test]
fn a_miss_under_the_release_policy_stops_capturing_until_released() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    frame(&mut cache, &mut r, fresh(key(1)), StaticLayer::Missed);
    assert_eq!(cache.plan(&mut r, fresh(key(2))), LayerPlan::Ignore);
    assert!(!cache.is_on());
    cache.release(&mut r);
    assert_eq!(
        cache.plan(&mut r, fresh(key(2))),
        LayerPlan::Capture(key(2))
    );
}

#[test]
fn a_miss_under_the_next_tree_policy_stops_capturing_until_a_fresh_tree() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::NextTree);
    frame(&mut cache, &mut r, fresh(key(1)), StaticLayer::Missed);
    assert_eq!(cache.plan(&mut r, replay(key(1))), LayerPlan::Ignore);
    assert_eq!(
        cache.plan(&mut r, fresh(key(1))),
        LayerPlan::Capture(key(1))
    );
}

#[test]
fn a_forgotten_layer_is_captured_again() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    cache.forget();
    assert_eq!(
        cache.plan(&mut r, replay(key(1))),
        LayerPlan::Capture(key(1))
    );
}

#[test]
fn release_frees_the_layer_and_forgets_the_empty_key() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    frame(
        &mut cache,
        &mut r,
        fresh(key(1)),
        StaticLayer::Captured { empty: true },
    );
    assert!(r.begin_static_layer(&owner(), 100, 20));
    cache.release(&mut r);
    assert!(r.static_layer_owners().is_empty());
    assert_eq!(
        cache.plan(&mut r, fresh(key(1))),
        LayerPlan::Capture(key(1))
    );
}

#[test]
fn release_leaves_a_tree_without_a_layer_without_one() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::NextTree);
    let bare = FrameTree {
        has_static_content: false,
        ..fresh(key(1))
    };
    assert_eq!(cache.plan(&mut r, bare), LayerPlan::Ignore);
    cache.release(&mut r);
    assert!(
        !cache.is_on(),
        "a host would preserve a target the next frame paints without a layer"
    );
}

#[test]
fn an_asset_change_moves_the_key() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let restored = LayerKey {
        assets: AssetGeneration::Ledger(1),
        ..key(1)
    };
    assert_eq!(
        cache.plan(&mut r, replay(restored)),
        LayerPlan::Capture(restored)
    );
}

#[test]
fn a_rekeyed_capture_is_reused_under_its_new_assets() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    let walked = AssetGeneration::Ledger(3);
    let plan = cache.plan(&mut r, fresh(key(1)));
    cache.commit(&mut r, plan, CAPTURED, walked);
    let now = LayerKey {
        assets: walked,
        ..key(1)
    };
    assert_eq!(cache.plan(&mut r, replay(now)), LayerPlan::Reuse(now));
}

#[test]
fn a_reuse_the_walk_captured_is_keyed_by_its_walked_assets() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let walked = AssetGeneration::Ledger(5);
    let plan = cache.plan(&mut r, replay(key(1)));
    assert_eq!(plan, LayerPlan::Reuse(key(1)));
    // As when the layer was stale against the frame's background, so the walk captured it.
    cache.commit(&mut r, plan, CAPTURED, walked);
    let now = LayerKey {
        assets: walked,
        ..key(1)
    };
    assert_eq!(cache.plan(&mut r, replay(now)), LayerPlan::Reuse(now));
}

#[test]
fn a_commit_reports_what_the_frame_did_to_the_static_half() {
    let commit = |plan_of: fn(LayerKey) -> FrameTree, layer| {
        let mut r = layers();
        let mut cache = captured(MissPolicy::Release, &mut r, key(1));
        let plan = cache.plan(&mut r, plan_of(key(1)));
        cache.commit(&mut r, plan, layer, AssetGeneration::Unchanging)
    };
    assert_eq!(
        cache(MissPolicy::Release).commit(
            &mut layers(),
            LayerPlan::Ignore,
            StaticLayer::Unused,
            AssetGeneration::Unchanging
        ),
        LayerCommit::Unlayered
    );
    assert_eq!(commit(replay, StaticLayer::Blitted), LayerCommit::Reused);
    assert_eq!(commit(replay, StaticLayer::Missed), LayerCommit::Missed);
    assert_eq!(
        commit(replay, CAPTURED),
        LayerCommit::Rewritten,
        "a stale layer turns the planned reuse into a capture"
    );
    let scrolling = |key| FrameTree {
        scrolling: true,
        ..replay(key)
    };
    assert_eq!(
        commit(scrolling, StaticLayer::Unused),
        LayerCommit::Rewritten,
        "a planned capture draws the static half anew even when it found no layer"
    );
}

fn panel() -> Node {
    Node::Column(
        PropsData {
            background: Color::from_rgb(1, 2, 3),
            ..PropsData::default()
        },
        Vec::new(),
    )
}

#[test]
fn a_drag_over_a_tree_without_a_scroll_container_scrolls_nothing() {
    let frame = FrameTree::new(&panel(), Some(key(1)), true, false);
    assert!(
        !frame.scrolling,
        "a finger jittering on a button keeps the layer"
    );
}

#[test]
fn a_drag_over_a_scroll_container_scrolls() {
    let list = bmc_ui::scroll("list", PropsData::default(), [panel()]);
    assert!(FrameTree::new(&list, Some(key(1)), true, false).scrolling);
}

#[test]
fn a_scroll_container_without_a_drag_scrolls_nothing() {
    let list = bmc_ui::scroll("list", PropsData::default(), [panel()]);
    assert!(!FrameTree::new(&list, Some(key(1)), false, false).scrolling);
}

#[test]
fn a_capture_may_leave_a_layer_until_released() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    assert!(
        !cache.may_hold_layer(),
        "nothing is captured before the first frame"
    );
    frame(&mut cache, &mut r, fresh(key(1)), CAPTURED);
    assert!(cache.may_hold_layer());
    cache.release(&mut r);
    assert!(!cache.may_hold_layer());
}

#[test]
fn a_failed_walk_may_still_leave_a_layer() {
    let mut r = layers();
    let mut cache = cache(MissPolicy::Release);
    let _ = cache.plan(&mut r, fresh(key(1)));
    cache.forget();
    assert!(
        cache.may_hold_layer(),
        "the walk may have begun the capture before it failed"
    );
}

#[test]
fn a_tree_without_a_layer_frees_what_the_last_one_left() {
    let mut r = layers();
    let mut cache = captured(MissPolicy::Release, &mut r, key(1));
    let unkeyed = FrameTree {
        key: None,
        ..fresh(key(1))
    };
    let _ = cache.plan(&mut r, unkeyed);
    assert!(!cache.may_hold_layer());
}
