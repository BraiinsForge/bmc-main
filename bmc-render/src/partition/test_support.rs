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

//! Edits to a host-built tree for testing its [`super::tree_static_key`].

use bmc_ui::{AnimProperty, AnimationDef, Color, ColorSpace, Easing, LoopMode};

use crate::tree::{Draw, Node};

/// Change every dynamic node and dynamic draw in `node`, and nothing static.
///
/// Returns how many it changed, so a test can tell a held key
/// from a tree that had nothing dynamic to move.
pub fn move_dynamic_content(node: &mut Node) -> usize {
    match node {
        Node::Column(_, children)
        | Node::Row(_, children)
        | Node::Center(_, children)
        | Node::Scroll { children, .. } => children.iter_mut().map(move_dynamic_content).sum(),
        Node::Tag { content, .. } => move_dynamic_content(content),
        Node::Dimmed { child, .. } => move_dynamic_content(child),
        Node::Canvas { draws, .. } => draws
            .iter_mut()
            .map(|draw| usize::from(move_dynamic_draw(draw)))
            .sum(),
        Node::RelTime { anchor, .. } => {
            *anchor += 1;
            1
        }
        Node::ProgressBar { active, .. }
        | Node::Modal {
            is_open: active, ..
        } => {
            *active = !*active;
            1
        }
        Node::Paragraph { .. }
        | Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::Notification { .. } => 0,
    }
}

fn move_dynamic_draw(draw: &mut Draw) -> bool {
    match draw {
        Draw::Modified {
            animations,
            transition,
            inner,
            ..
        } => {
            if let Some(animation) = animations.first_mut() {
                animation.to += 1.0;
                true
            } else if let Some(transition) = transition {
                transition.duration_ms += 1;
                true
            } else {
                move_dynamic_draw(inner)
            }
        }
        Draw::Centered { inner }
        | Draw::Orbit { inner, .. }
        | Draw::Rotated { inner, .. }
        | Draw::Shadow { inner, .. } => move_dynamic_draw(inner),
        Draw::Rect { .. }
        | Draw::Circle { .. }
        | Draw::Arc { .. }
        | Draw::Svg { .. }
        | Draw::Bitmap { .. }
        | Draw::Path { .. }
        | Draw::Sphere { .. }
        | Draw::Mesh { .. }
        | Draw::Text { .. }
        | Draw::CurvedText { .. }
        | Draw::AutofitText { .. }
        | Draw::NinePatch { .. }
        | Draw::Qr { .. } => false,
    }
}

/// Append a spinning draw to the first canvas in `node`,
/// for a tree that animates nothing yet.
///
/// Returns whether `node` had a canvas to take it.
pub fn add_animated_draw(node: &mut Node) -> bool {
    match node {
        Node::Column(_, children)
        | Node::Row(_, children)
        | Node::Center(_, children)
        | Node::Scroll { children, .. } => children.iter_mut().any(add_animated_draw),
        Node::Tag { content, .. } => add_animated_draw(content),
        Node::Dimmed { child, .. } => add_animated_draw(child),
        Node::Canvas { draws, .. } => {
            draws.push(spinner());
            true
        }
        // A modal's body is dynamic already, so a spinner there would prove nothing.
        Node::Modal { .. }
        | Node::RelTime { .. }
        | Node::ProgressBar { .. }
        | Node::Paragraph { .. }
        | Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::Notification { .. } => false,
    }
}

fn spinner() -> Draw {
    Draw::Modified {
        animations: vec![AnimationDef {
            property: AnimProperty::Rotate,
            from: 0.0,
            to: 360.0,
            duration_ms: 1000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(Draw::rect(0.0, 0.0, 10.0, 10.0, Color::from_rgb(1, 2, 3))),
    }
}
