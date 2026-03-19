// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    let merge_dir = gen_path.join("merge-files");

    if !merge_dir.exists() {
        return Ok(());
    }

    // Collect all leaf files under merge-files/, grouped by their parent directory
    // (which becomes the target path in the generation root).
    let mut groups: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

    for entry in WalkDir::new(&merge_dir).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&merge_dir)
                .expect("BUG: entry must be under merge_dir");
            let target = rel
                .parent()
                .expect("BUG: file must have a parent directory");
            groups
                .entry(target.to_path_buf())
                .or_default()
                .push(entry.path().to_path_buf());
        }
    }

    for (target, mut files) in groups {
        files.sort();

        let mut content = String::new();
        for file in &files {
            content.push_str(&std::fs::read_to_string(file)?);
        }

        let output_path = gen_path.join(&target);

        // Verify the output path doesn't escape the generation directory
        // (e.g. via `..` components in merge-files/ entries).
        let canonical_gen = gen_path.canonicalize().map_err(|e| {
            anyhow::anyhow!(
                "failed to canonicalize generation path '{}': {e}",
                gen_path.display()
            )
        })?;
        let canonical_output = if output_path.exists() {
            output_path.canonicalize()?
        } else {
            // Parent must exist for canonicalize; resolve the parent and append the filename
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)?;
                parent.canonicalize()?.join(
                    output_path
                        .file_name()
                        .expect("BUG: output_path must have a filename"),
                )
            } else {
                output_path.clone()
            }
        };

        anyhow::ensure!(
            canonical_output.starts_with(&canonical_gen),
            "merge-files output path escapes generation directory: {}",
            output_path.display()
        );

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&output_path, &content)?;
    }

    Ok(())
}
