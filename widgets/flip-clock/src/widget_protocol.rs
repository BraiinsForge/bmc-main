// Copyright (C) 2025  Braiins Systems s.r.o.

//! Decode widget-specific params from the `deck_widget_v1` configure batch.
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

/// Manifest-declared parameters for the flip-clock widget.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ManifestParams {
    mode: Option<AnimationModeKind>,
    timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnimationModeKind {
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

/// Resolve flip-clock's animation mode from the protocol-delivered
/// params. Missing → default; unknown string → decode error.
pub fn animation_mode_from_params(params: &serde_json::Value) -> Result<AnimationMode, IpcError> {
    let parsed: ManifestParams = serde_json::from_value(params.clone())?;
    Ok(parsed
        .mode
        .map_or_else(AnimationMode::default, AnimationMode::from))
}

/// Resolve an optional per-widget timezone override from the params.
///
/// Returns `None` when the widget has no override — the caller must use
/// the system timezone delivered via the `timezone` setting event.
pub fn timezone_override_from_params(
    params: &serde_json::Value,
) -> Result<Option<String>, IpcError> {
    let parsed: ManifestParams = serde_json::from_value(params.clone())?;
    Ok(parsed.timezone)
}
