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

//! Whether a frame captures its tree's static half into a layer, reuses the layer, or paints in full.
//!
//! The widget host and the overlays follow the same rules, so both drive a [`StaticLayerCache`]
//! and keep around it only what is theirs: how they key a tree,
//! and what a frame owes the buffers it does not paint.

use super::{
    HostPaintState, LayoutKey, StaticBytesKey, StaticHash, has_scroll_container,
    has_static_content, host_layout_key, static_hash,
};
use crate::renderer::{LayerOwner, Renderer};
use crate::tree::{LayerUse, Node, StaticLayer};

/// The state of a caller's asset registrations when a layer is keyed.
///
/// A tag re-registered with different bytes keys the tree the same while what it paints changed,
/// so the generation is part of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetGeneration {
    /// For a caller that never registers different bytes under a tag it has drawn.
    Unchanging,
    /// The renderer asset ledger's generation.
    Ledger(u64),
}

/// Everything a captured layer's pixels depend on:
/// two frames with one key paint the same static half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerKey {
    /// The surface the layer is captured at, in logical pixels.
    pub size: (u32, u32),
    pub static_hash: StaticHash,
    pub layout: LayoutKey,
    pub assets: AssetGeneration,
}

impl LayerKey {
    /// The key of `node`, whose wire bytes key as `static_bytes`, laid out at `size`,
    /// painted as `host` paints it and worded by `intl`.
    #[must_use]
    pub fn new(
        node: &Node,
        static_bytes: StaticBytesKey,
        host: &HostPaintState<'_>,
        size: (u32, u32),
        assets: AssetGeneration,
        now_unix_secs: i64,
        intl: &bmc_intl::Intl,
    ) -> Self {
        Self {
            size,
            static_hash: static_hash(static_bytes, host),
            layout: host_layout_key(node, now_unix_secs, intl),
            assets,
        }
    }
}

/// What a frame does with the layer, and the key it captures or reuses it under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPlan {
    Ignore,
    Capture(LayerKey),
    Reuse(LayerKey),
}

/// What a committed frame did to the static half it drew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerCommit {
    /// The frame took no layer.
    Unlayered,
    /// The frame drew the static half anew, as a capture planned or forced by a stale layer.
    Rewritten,
    /// The frame blitted the static half it planned to reuse.
    Reused,
    /// The frame planned to reuse the static half, missed the blit and painted the tree in full.
    Missed,
}

/// When capturing starts again after a miss.
///
/// On a driver whose layer copy fails, every capture misses and costs a static pass on top of a full paint,
/// so a caller whose trees change often waits for a release instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissPolicy {
    /// At the next freshly built tree.
    NextTree,
    /// At [`StaticLayerCache::release`].
    Release,
}

/// What the caller knows about the frame's tree.
#[derive(Debug, Clone, Copy)]
pub struct FrameTree {
    /// The tree's layer key, `None` for a tree the wire format cannot key.
    pub key: Option<LayerKey>,
    /// Per [`super::has_static_content`].
    pub has_static_content: bool,
    /// Whether a drag moves a scroll container during this frame's walk,
    /// after the key was taken.
    pub scrolling: bool,
    /// Built for this frame, not a replay of the last one.
    pub fresh: bool,
}

impl FrameTree {
    /// What a frame of `node` keyed as `key` brings to [`StaticLayerCache::plan`],
    /// `dragging` when a finger moved this frame.
    #[must_use]
    pub fn new(node: &Node, key: Option<LayerKey>, dragging: bool, fresh: bool) -> Self {
        Self {
            key,
            has_static_content: has_static_content(node),
            // A scroll container advances its offset inside the walk,
            // so a key taken before it holds the offset the previous frame ended on.
            // A tree without one has nothing a drag moves,
            // so a finger jittering on a button keeps the layer.
            scrolling: dragging && has_scroll_container(node),
            fresh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The tree takes no layer: it has no static content or no key,
    /// or keys like a capture that came out empty.
    Off,
    /// The tree takes a layer, but none holds its static half: the next frame captures.
    Pending,
    /// The layer holds the static half captured under this key.
    Held(LayerKey),
    /// A miss stopped capturing until the [`MissPolicy`] lets it start again.
    Stopped,
}

/// One tree's static layer in the renderer, and the rules for using it.
#[derive(Debug)]
pub struct StaticLayerCache {
    owner: LayerOwner,
    state: State,
    /// The key whose capture came out empty, so trees keyed like it go without a layer.
    empty: Option<LayerKey>,
    policy: MissPolicy,
    /// Whether a walk was handed the layer since it was last freed,
    /// which leaves one in the renderer even when the walk failed before its commit.
    may_hold: bool,
}

impl StaticLayerCache {
    #[must_use]
    pub fn new(owner: LayerOwner, policy: MissPolicy) -> Self {
        Self {
            owner,
            state: State::Pending,
            empty: None,
            policy,
            may_hold: false,
        }
    }

    /// Whose layer this is in the renderer.
    #[must_use]
    pub fn owner(&self) -> &LayerOwner {
        &self.owner
    }

    /// What the walk is asked to do with the layer for `plan`.
    #[must_use]
    pub fn layer_use(&self, plan: LayerPlan) -> LayerUse<'_> {
        match plan {
            LayerPlan::Ignore => LayerUse::Ignore,
            LayerPlan::Capture(_) => LayerUse::Capture(&self.owner),
            LayerPlan::Reuse(_) => LayerUse::Reuse(&self.owner),
        }
    }

    /// Whether the current tree takes a layer.
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self.state, State::Pending | State::Held(_))
    }

    /// Whether the renderer may hold a layer of this cache's owner, which only freeing it clears.
    #[must_use]
    pub fn may_hold_layer(&self) -> bool {
        self.may_hold
    }

    /// What this frame does with the layer.
    ///
    /// A tree that takes no layer frees the one the cache held,
    /// or a later tree keyed like it would reuse a layer that is gone.
    pub fn plan(&mut self, renderer: &mut dyn Renderer, tree: FrameTree) -> LayerPlan {
        if tree.fresh && self.policy == MissPolicy::NextTree && self.state == State::Stopped {
            self.state = State::Pending;
        }
        if self.state == State::Stopped {
            return LayerPlan::Ignore;
        }
        let Some(key) = tree
            .key
            .filter(|key| tree.has_static_content && self.empty != Some(*key))
        else {
            self.drop_layer(renderer);
            self.state = State::Off;
            return LayerPlan::Ignore;
        };
        if self.state == State::Off {
            self.state = State::Pending;
        }
        self.may_hold = true;
        if self.state == State::Held(key) && !tree.scrolling {
            LayerPlan::Reuse(key)
        } else {
            LayerPlan::Capture(key)
        }
    }

    /// Record what the walk left in the layer for `plan`.
    ///
    /// `walked_assets` is the asset generation after the walk. An asset the walk restored
    /// moved it, and a capture holds the restored pixels, so it keys the captured layer,
    /// also when the walk captured a stale layer it was planned to reuse.
    pub fn commit(
        &mut self,
        renderer: &mut dyn Renderer,
        plan: LayerPlan,
        layer: StaticLayer,
        walked_assets: AssetGeneration,
    ) -> LayerCommit {
        let planned = match plan {
            LayerPlan::Capture(key) | LayerPlan::Reuse(key) => key,
            LayerPlan::Ignore => return LayerCommit::Unlayered,
        };
        let captured = LayerKey {
            assets: walked_assets,
            ..planned
        };
        match layer {
            StaticLayer::Unused => {}
            // Such a layer only costs a blit, and dropping it never changes the picture.
            StaticLayer::Captured { empty: true } => {
                self.drop_layer(renderer);
                self.empty = Some(captured);
                self.state = State::Off;
            }
            StaticLayer::Captured { empty: false } => self.state = State::Held(captured),
            StaticLayer::Blitted => self.state = State::Held(planned),
            StaticLayer::Missed => {
                self.drop_layer(renderer);
                self.state = State::Stopped;
            }
        }
        if matches!(plan, LayerPlan::Capture(_)) || matches!(layer, StaticLayer::Captured { .. }) {
            LayerCommit::Rewritten
        } else if layer == StaticLayer::Missed {
            LayerCommit::Missed
        } else {
            LayerCommit::Reused
        }
    }

    /// Stop counting on the layer, which the next capture overwrites,
    /// as after a frame whose walk failed.
    pub fn forget(&mut self) {
        if let State::Held(_) = self.state {
            self.state = State::Pending;
        }
    }

    /// Free the layer and capture again from the next frame, even after a miss stopped it.
    /// A tree that takes no layer still takes none until the next plan says otherwise.
    /// Needs the renderer's GL context current.
    pub fn release(&mut self, renderer: &mut dyn Renderer) {
        renderer.invalidate_static_layer(&self.owner);
        self.may_hold = false;
        self.empty = None;
        if self.state != State::Off {
            self.state = State::Pending;
        }
    }

    fn drop_layer(&mut self, renderer: &mut dyn Renderer) {
        renderer.invalidate_static_layer(&self.owner);
        self.may_hold = false;
        self.forget();
    }
}

#[cfg(test)]
mod tests;
