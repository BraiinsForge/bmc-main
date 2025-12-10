// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources({
                if cfg!(feature = "slint-embed-files") {
                    slint_build::EmbedResourcesKind::EmbedFiles
                } else {
                    slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer
                }
            })
            .with_library_paths(
                [(
                    "bmc-shared-slint".into(),
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../bmc-shared/slint/ui/lib.slint"),
                )]
                .into(),
            ),
    )
    .context("BUG: Slint compilation error")?;

    Ok(())
}
