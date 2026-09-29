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

//! The views, their shared parts and the fixtures that stage them.

mod bmm101;
pub mod fixtures;
mod parts;
mod view;

pub use view::{ViewData, launch_view};

#[cfg(test)]
mod tree {
    use bmc_wasm_sdk::{Draw, Node, TextOverflow};

    /// How the paragraph drawing `content` handles running out of room.
    pub(super) fn overflow_of(node: &Node, content: &str) -> Option<TextOverflow> {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children
                    .iter()
                    .find_map(|child| overflow_of(child, content))
            }
            Node::Paragraph {
                base_style, spans, ..
            } => spans
                .iter()
                .any(|span| span.text == content)
                .then_some(base_style.text_overflow),
            _ => None,
        }
    }

    /// Every string the tree would draw, in tree order, canvas text included.
    pub(super) fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            Node::Canvas { draws, .. } => {
                for draw in draws {
                    if let Draw::AutofitText { text, .. } = draw {
                        out.push(text.clone());
                    }
                }
            }
            _ => {}
        }
    }
}
