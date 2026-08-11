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

//! `preview` subcommand — generate mp4 preview videos from captured frames.
//!
//! Shells out to `ffmpeg` to encode `frame_%04d.png` sequences into H.264 mp4
//! files. Operates on a single widget's output directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use owo_colors::OwoColorize;

use super::tools::resolve_tool;

// ── Public interface ────────────────────────────────────────────────

pub struct PreviewArgs {
    /// Path to the widget's output directory (contains `current/`).
    pub output: PathBuf,
    /// Only generate previews for this target (e.g. "bmc100:full").
    pub target: Option<String>,
    /// Frame rate for the output video.
    pub fps: u32,
}

pub fn execute(args: &PreviewArgs) -> Result<()> {
    let ffmpeg_bin = resolve_tool("ffmpeg", "ffmpeg")?;

    let current_dir = args.output.join("current");
    if !current_dir.is_dir() {
        eprintln!(
            "  {} (no captures found at {})",
            "skip".dimmed(),
            current_dir.display().dimmed()
        );
        return Ok(());
    }

    // Walk current/ to find directories containing frame_0000.png
    let dirs = find_frame_dirs(&current_dir)?;
    if dirs.is_empty() {
        eprintln!(
            "  {} (no frame sequences found in {})",
            "skip".dimmed(),
            current_dir.display().dimmed()
        );
        return Ok(());
    }

    for dir in &dirs {
        let rel = dir
            .strip_prefix(&current_dir)
            .expect("BUG: frame dir not under current/");
        let label = rel.to_string_lossy();

        // Frames live under <platform>/<viewport>/<dataset>,
        // so a target filter matches the leading two components.
        if let Some(filter) = args.target.as_deref() {
            let prefix = filter.replace(':', "/");
            if !label.starts_with(&prefix) {
                continue;
            }
        }

        let output_name = format!("preview_{}.mp4", label.replace('/', "_"));
        let output_path = args.output.join(&output_name);

        encode_video(&ffmpeg_bin, dir, &output_path, args.fps)
            .with_context(|| format!("failed to encode preview for {label}"))?;

        eprintln!(
            "  {} {} {}",
            "✓".green(),
            label.green(),
            output_path.display().dimmed()
        );
    }

    Ok(())
}

// ── Frame directory discovery ───────────────────────────────────────

/// Find all directories under `base` that contain `frame_0000.png`.
fn find_frame_dirs(base: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    collect_frame_dirs(base, &mut dirs)?;
    dirs.sort();
    Ok(dirs)
}

fn collect_frame_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if dir.join("frame_0000.png").exists() {
        out.push(dir.to_owned());
    }
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_frame_dirs(&path, out)?;
        }
    }
    Ok(())
}

// ── ffmpeg encoding ─────────────────────────────────────────────────

fn encode_video(ffmpeg: &Path, frame_dir: &Path, output: &Path, fps: u32) -> Result<()> {
    let input_pattern = frame_dir.join("frame_%04d.png");

    // Ensure parent directory exists
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let status = Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-y",
            "-loglevel",
            "warning",
            "-framerate",
            &fps.to_string(),
            "-i",
        ])
        .arg(&input_pattern)
        // libx264 rejects odd dimensions; the `small` capture size is 317x238
        // (odd width), so pad each axis up to the next even number (≤1px, black).
        .args([
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(output)
        .status()
        .context("failed to spawn ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg exited with code {}", status.code().unwrap_or(-1));
    }

    Ok(())
}
