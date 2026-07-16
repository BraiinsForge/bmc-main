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

use bmc_wasm_protocol::{SvgId, TagKind};

use crate::tree::Node;

/// Leading icon for a [`tag`]. `Default` uses the per-kind theme icon, `Hidden`
/// draws none, `Custom` overrides with an explicit glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagIcon {
    Default,
    Hidden,
    Custom(SvgId),
}

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
    use bmc_wasm_protocol::{ICON_WARN_FILLED, NODE_TAG, TagIconMode, TagKind};

    use super::TagIcon;
    use crate::tree::{Node, TreeBuffer};

    fn chrome_bytes(kind: TagKind, icon: TagIcon) -> Vec<u8> {
        let mut buf = TreeBuffer::new();
        buf.write_tag(kind, icon);
        buf.into_bytes()
    }

    #[test]
    fn wire_leads_with_type_kind_icon() {
        let bytes = chrome_bytes(TagKind::Warning, TagIcon::Default);
        assert_eq!(bytes[0], NODE_TAG);
        assert_eq!(bytes[1], TagKind::Warning as u8);
        assert_eq!(bytes[2], TagIconMode::Default as u8);
    }

    #[test]
    fn icon_mode_encodes_all_three_states() {
        assert_eq!(
            chrome_bytes(TagKind::Info, TagIcon::Default)[2],
            TagIconMode::Default as u8
        );
        assert_eq!(
            chrome_bytes(TagKind::Info, TagIcon::Hidden)[2],
            TagIconMode::Hidden as u8
        );

        let custom = chrome_bytes(TagKind::Info, TagIcon::Custom(ICON_WARN_FILLED));
        assert_eq!(custom[2], TagIconMode::Custom as u8);
        assert_eq!(&custom[3..5], &ICON_WARN_FILLED.to_wire().to_le_bytes());
    }

    #[test]
    fn builder_boxes_the_content_child() {
        let node = super::tag(TagKind::Error, TagIcon::Hidden, Node::Spacer { flex: 1.0 });
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
