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

//! Decoding the guest's wire-format tree into [`TreeNode`]s.

use super::TreeNode;
use anyhow::Result;

/// Deserialize a tree from bytes
pub fn deserialize_tree(data: &[u8]) -> Result<TreeNode> {
    Ok(decode_logged(data)?.root)
}

/// Deserialize a tree from bytes,
/// along with the [`crate::partition::static_bytes_key`] of its static half.
pub fn deserialize_tree_keyed(data: &[u8]) -> Result<(TreeNode, u64)> {
    let decoded = decode_logged(data)?;
    let key =
        crate::partition::static_bytes_key(&data[..decoded.consumed], &decoded.dynamic_ranges);
    Ok((decoded.root, key))
}

fn decode_logged(data: &[u8]) -> Result<bmc_ui::Decoded> {
    let decoded = bmc_ui::decode(data)?;
    let trailing = data.len() - decoded.consumed;
    if trailing > 0 {
        tracing::warn!("tree has {trailing} trailing bytes");
    }
    Ok(decoded)
}
