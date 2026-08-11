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

//! `set-baseline` subcommand — compress current captures into baselines.7z.
//!
//! Operates on a single widget. The orchestrator resolves paths and calls
//! this for each widget.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result};
use owo_colors::OwoColorize;
use sevenz_rust2::encoder_options::Lzma2Options;

// ── Public interface ────────────────────────────────────────────────

pub struct SetBaselineArgs {
    /// Path to the `capture/` directory — baselines.7z is written here.
    pub capture_dir: PathBuf,
    /// Path to the widget's output directory (contains `current/`).
    pub output: PathBuf,
}

pub fn execute(args: &SetBaselineArgs) -> Result<()> {
    let current_dir = args.output.join("current");
    if !current_dir.is_dir() {
        eprintln!(
            "  {} (no captures found at {})",
            "skip".dimmed(),
            current_dir.display().dimmed()
        );
        return Ok(());
    }

    let baseline_path = args.capture_dir.join("baselines.7z");

    // Remove old baseline
    if baseline_path.exists() {
        std::fs::remove_file(&baseline_path).with_context(|| {
            format!("failed to remove old baseline {}", baseline_path.display())
        })?;
    }

    // Ensure parent directory exists
    if let Some(parent) = baseline_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let t0 = Instant::now();

    // Compress current/ → baselines.7z using solid LZMA2 compression (max level)
    compress_directory(&current_dir, &baseline_path).with_context(|| {
        format!(
            "failed to compress {} → {}",
            current_dir.display(),
            baseline_path.display()
        )
    })?;

    let elapsed = t0.elapsed().as_secs_f64();
    #[expect(clippy::cast_precision_loss)]
    let size_kb = std::fs::metadata(&baseline_path).map_or(0.0, |m| m.len() as f64 / 1_024.0);

    eprintln!(
        "  {} {} {} {}",
        "✓".green(),
        baseline_path.display().green(),
        format!("({size_kb:.0}K)").dimmed(),
        format!("{elapsed:.1}s").dimmed()
    );

    Ok(())
}

// ── 7z compression ──────────────────────────────────────────────────

/// Compress a directory into a solid 7z archive with max LZMA2 compression.
fn compress_directory(source_dir: &Path, archive_path: &Path) -> Result<()> {
    use sevenz_rust2::ArchiveWriter;

    let file = std::fs::File::create(archive_path)?;
    let mut writer = ArchiveWriter::new(file)?;

    // LZMA2 level 9, 4 threads, 8MB dictionary (matches `7z -mx=9` output)
    let options = Lzma2Options::from_level_mt(9, 4, 1 << 23);
    writer.set_content_methods(vec![options.into()]);

    // Solid mode — all files in a single block for best compression of similar PNGs
    writer.push_source_path(source_dir, |_| true)?;
    writer.finish()?;

    Ok(())
}
