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

//! Decode widget-specific params from the `deck_widget` configure batch.
//!
//! The compositor delivers every piece of initial state as typed
//! protocol events on surface creation. This module turns the relevant
//! slice of that state (flip-clock's `mode` param) into the widget's
//! internal [`AnimationMode`].

use serde::Deserialize;

use crate::AnimationMode;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("param decode error: {0}")]
    Params(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestParams {
    pub mode: AnimationModeKind,
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationModeKind {
    Flat,
    Extruded,
}

impl From<AnimationModeKind> for AnimationMode {
    fn from(k: AnimationModeKind) -> Self {
        match k {
            AnimationModeKind::Flat => Self::Flat,
            AnimationModeKind::Extruded => Self::Extruded,
        }
    }
}

pub fn animation_mode_from_params(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<AnimationMode, IpcError> {
    let parsed: ManifestParams = serde_json::from_value(params.clone().into())?;
    Ok(parsed.mode.into())
}

/// Resolve an optional per-widget timezone override from the params.
///
/// Returns `None` when the widget has no override — the caller must use
/// the system timezone delivered via the `timezone` setting event.
pub fn timezone_override_from_params(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<String>, IpcError> {
    let parsed: ManifestParams = serde_json::from_value(params.clone().into())?;
    Ok(parsed.timezone)
}
