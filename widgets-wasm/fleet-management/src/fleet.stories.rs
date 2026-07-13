// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fleet screen stories, rendered natively (same path as the wasm widget).

use crate::prelude::*;
use bmc_wasm_sdk::SizeVariant;
use fleet_management::screens;

story_meta! { title: "Fleet" }

#[story(default)]
fn table(_ctx: &mut StoryCtx) -> Node {
    let summary = screens::fixtures::sample_fleet();
    screens::view(&summary, None, 0, 638, 480, SizeVariant::Large, "My Fleet").root
}
