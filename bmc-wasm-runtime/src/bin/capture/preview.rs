// Copyright (C) 2026  Braiins Systems s.r.o.

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
    /// Only generate previews for this size (e.g. "full").
    pub size: Option<String>,
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
        // The relative path from current/ gives us the size/variant label
        let rel = dir
            .strip_prefix(&current_dir)
            .expect("BUG: frame dir not under current/");
        let label = rel.to_string_lossy();

        // Apply size filter if given
        if let Some(filter) = args.size.as_deref() {
            let parts: Vec<&str> = label.split('/').collect();
            let size_part = parts.last().expect("BUG: empty path components");
            if *size_part != filter {
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
            "-y",
            "-loglevel",
            "warning",
            "-framerate",
            &fps.to_string(),
            "-i",
        ])
        .arg(&input_pattern)
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(output)
        .status()
        .context("failed to spawn ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg exited with code {}", status.code().unwrap_or(-1));
    }

    Ok(())
}
