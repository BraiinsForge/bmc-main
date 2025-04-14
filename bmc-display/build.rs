// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    let config = slint_build::CompilerConfiguration::new();

    #[cfg(not(feature = "standalone"))]
    config.embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);

    slint_build::compile_with_config("assets/ui/main.slint", config)
        .context("BUG: Slint compilation error")?;

    Ok(())
}
