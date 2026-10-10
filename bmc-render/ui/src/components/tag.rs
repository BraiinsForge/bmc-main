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

//! Tag pill builder — a Carbon-style status label.

use crate::nodes::TagKind;
use crate::tree::{Node, TagIcon};

/// Carbon status pill: rounded [`TagKind`]-themed chrome (background + leading
/// icon, host-rendered) wrapping the embedder-composed `content` node.
#[must_use]
pub fn tag(kind: TagKind, icon: TagIcon, content: Node) -> Node {
    Node::Tag {
        kind,
        icon,
        content: Box::new(content),
    }
}

#[cfg(test)]
mod tests {
    use super::{TagIcon, TagKind, tag};
    use crate::tree::Node;

    #[test]
    fn builder_boxes_the_content_child() {
        let node = tag(TagKind::Error, TagIcon::Hidden, Node::Spacer { flex: 1.0 });
        assert!(matches!(
            node,
            Node::Tag {
                kind: TagKind::Error,
                icon: TagIcon::Hidden,
                ..
            }
        ));
    }
}
