// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    slint_build::compile_with_config(
        "assets/ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .context("BUG: Slint compilation error")?;

    Ok(())
}
