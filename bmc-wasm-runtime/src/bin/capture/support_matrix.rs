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

//! Which targets each widget's manifest admits, as a grid.
//!
//! Manifests only — no build, no wasm, no GL.
//! So this reports what a widget *declares*, not what its layout does there:
//! a manifest declaring a size range admits every geometry inside it,
//! including ones no layout was ever drawn for.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bmc_wasm_runtime::platform_catalog::{
    self, PLATFORMS, Platform, Target, manifest_viewport_shape,
};
use bmc_widget_manifest::{Manifest, ViewportDeclined};

use crate::run_all::{discover_widgets, workspace_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Aligned columns for a terminal.
    Table,
    /// Pipe table, for pasting into docs and tickets.
    Markdown,
    /// Machine-readable, for scripting over the grid.
    Json,
}

pub struct SupportMatrixArgs {
    pub workspaces: Vec<PathBuf>,
    /// Narrow the sweep to one platform, by catalog id.
    pub platform: Option<String>,
    pub format: Format,
}

/// A manifest's verdict on one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Support {
    Admitted,
    DeclinedGeometry,
    DeclinedDpi,
}

impl Support {
    /// Cell text, kept short so the table stays narrow enough to read.
    const fn cell(self) -> &'static str {
        match self {
            Self::Admitted => "yes",
            Self::DeclinedGeometry => "-",
            Self::DeclinedDpi => "dpi",
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::DeclinedGeometry => "declined:geometry",
            Self::DeclinedDpi => "declined:dpi",
        }
    }
}

struct WidgetRow {
    /// Crate directory name — how every other subcommand addresses a widget.
    dir: String,
    /// The manifest's own `name`, which is what the operator UI shows.
    name: String,
    workspace: String,
    /// One verdict per target, in the same order as the sweep's targets.
    verdicts: Vec<Support>,
}

/// # Errors
/// When a workspace is unreadable, a manifest fails to parse,
/// or `--platform` names nothing in the catalog.
pub fn execute(args: &SupportMatrixArgs) -> Result<()> {
    let targets = sweep_targets(args.platform.as_deref())?;
    let rows = collect_rows(&args.workspaces, &targets)?;
    if rows.is_empty() {
        bail!("no widgets discovered — no crate in these workspaces carries a manifest.json");
    }

    match args.format {
        Format::Table => print_table(&rows, &targets),
        Format::Markdown => print_markdown(&rows, &targets),
        Format::Json => print_json(&rows, &targets)?,
    }
    Ok(())
}

/// Every target the catalog offers, or one platform's when named.
fn sweep_targets(platform_id: Option<&str>) -> Result<Vec<Target>> {
    let platforms: Vec<&'static Platform> = match platform_id {
        // `select` carries the valid alternatives in its error, which a bare lookup does not.
        Some(id) => vec![platform_catalog::select(Some(id))?],
        None => PLATFORMS.iter().collect(),
    };

    Ok(platforms
        .into_iter()
        .flat_map(|platform| {
            platform
                .viewports
                .iter()
                .map(move |viewport| Target { platform, viewport })
        })
        .collect())
}

fn collect_rows(workspaces: &[PathBuf], targets: &[Target]) -> Result<Vec<WidgetRow>> {
    if workspaces.is_empty() {
        bail!("--workspace: at least one workspace required");
    }

    let mut rows = Vec::new();
    for workspace in workspaces {
        for dir in discover_widgets(workspace)? {
            // Discovery keys off Cargo.toml, which also matches a workspace's
            // shared crates; only a manifest makes a crate a widget.
            let manifest_path = workspace.join(&dir).join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }

            let text = std::fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?;
            let manifest: Manifest = text
                .parse()
                .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", manifest_path.display()))?;

            rows.push(WidgetRow {
                verdicts: targets
                    .iter()
                    .map(|target| verdict(&manifest, *target))
                    .collect(),
                dir,
                name: manifest.name,
                workspace: workspace_label(workspace).to_owned(),
            });
        }
    }
    Ok(rows)
}

fn verdict(manifest: &Manifest, target: Target) -> Support {
    match manifest.admits_viewport_at_dpi(
        manifest_viewport_shape(target.viewport.shape),
        target.viewport.width,
        target.viewport.height,
        target.platform.display().dpi,
    ) {
        Ok(()) => Support::Admitted,
        Err(ViewportDeclined::Geometry) => Support::DeclinedGeometry,
        Err(ViewportDeclined::Dpi) => Support::DeclinedDpi,
    }
}

/// Target column headers, each `<platform>:<viewport>` over its pixel size.
fn column_headers(targets: &[Target]) -> Vec<(String, String)> {
    targets
        .iter()
        .map(|target| {
            (
                target.to_string(),
                format!("{}x{}", target.viewport.width, target.viewport.height),
            )
        })
        .collect()
}

fn column_widths(rows: &[WidgetRow], headers: &[(String, String)]) -> (usize, Vec<usize>) {
    let widget_width = rows
        .iter()
        .map(|row| row.dir.len())
        .chain(std::iter::once("widget".len()))
        .max()
        .unwrap_or_default();

    let target_widths = headers
        .iter()
        .enumerate()
        .map(|(i, (target, size))| {
            rows.iter()
                .filter_map(|row| row.verdicts.get(i))
                .map(|verdict| verdict.cell().len())
                .chain([target.len(), size.len()])
                .max()
                .unwrap_or_default()
        })
        .collect();

    (widget_width, target_widths)
}

fn print_table(rows: &[WidgetRow], targets: &[Target]) {
    let headers = column_headers(targets);
    let (widget_width, target_widths) = column_widths(rows, &headers);

    print!("{:<widget_width$}", "widget");
    for ((target, _), width) in headers.iter().zip(&target_widths) {
        print!("  {target:>width$}");
    }
    println!();

    print!("{:<widget_width$}", "");
    for ((_, size), width) in headers.iter().zip(&target_widths) {
        print!("  {size:>width$}");
    }
    println!();

    for row in rows {
        print!("{:<widget_width$}", row.dir);
        for (verdict, width) in row.verdicts.iter().zip(&target_widths) {
            print!("  {:>width$}", verdict.cell());
        }
        println!();
    }

    println!();
    println!("yes = the manifest admits this geometry and density");
    println!("-   = no declared viewport takes this shape and size");
    println!("dpi = the size is declared, but not at this display's density");
    println!();
    println!("Declared support is not adapted layout: a manifest declaring a size");
    println!("range admits every geometry inside it, drawn for or not.");
}

fn print_markdown(rows: &[WidgetRow], targets: &[Target]) {
    let headers = column_headers(targets);

    let columns = headers
        .iter()
        .map(|(target, size)| format!("{target}<br>{size}"))
        .collect::<Vec<_>>()
        .join(" | ");
    println!("| widget | name | {columns} |");

    let divider = vec!["---"; headers.len() + 2].join(" | ");
    println!("| {divider} |");

    for row in rows {
        let cells = row
            .verdicts
            .iter()
            .map(|verdict| verdict.cell())
            .collect::<Vec<_>>()
            .join(" | ");
        println!("| `{}` | {} | {cells} |", row.dir, row.name);
    }
}

fn print_json(rows: &[WidgetRow], targets: &[Target]) -> Result<()> {
    let widgets: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let verdicts: serde_json::Map<String, serde_json::Value> = targets
                .iter()
                .zip(&row.verdicts)
                .map(|(target, verdict)| (target.to_string(), serde_json::json!(verdict.slug())))
                .collect();
            serde_json::json!({
                "widget": row.dir,
                "name": row.name,
                "workspace": row.workspace,
                "targets": verdicts,
            })
        })
        .collect();

    let doc = serde_json::json!({
        "targets": targets
            .iter()
            .map(|target| serde_json::json!({
                "target": target.to_string(),
                "width": target.viewport.width,
                "height": target.viewport.height,
                "dpi": target.platform.display().dpi,
            }))
            .collect::<Vec<_>>(),
        "widgets": widgets,
    });

    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_declaring(viewports: &str) -> Manifest {
        let json = format!(
            r#"{{
                "uid": "0a3973c9-3a97-4bf2-957a-741e55353a19",
                "version": "1.0.0",
                "name": "Test",
                "description": "Test widget",
                "author": {{ "name": "Braiins" }},
                "binary": "test.wasm",
                "category": "misc",
                "supported_viewports": {viewports}
            }}"#
        );
        json.parse().expect("BUG: the test manifest must parse")
    }

    fn bmm101_full() -> Target {
        "bmm101:full"
            .parse()
            .expect("BUG: bmm101:full must be in the catalog")
    }

    /// The Deck size range spans 480x320, so BMM101 is admitted by a widget
    /// that was only ever laid out for the Deck's four canonical sizes.
    #[test]
    fn a_deck_size_range_admits_bmm101() {
        let manifest = manifest_declaring(
            r#"[{ "type": "rectangular", "min_width": 317, "max_width": 1280,
                  "min_height": 238, "max_height": 480 }]"#,
        );

        assert_eq!(verdict(&manifest, bmm101_full()), Support::Admitted);
    }

    #[test]
    fn a_round_only_widget_declines_bmm101_on_geometry() {
        let manifest = manifest_declaring(
            r#"[{ "type": "round", "min_width": 480, "max_width": 480,
                  "min_height": 480, "max_height": 480 }]"#,
        );

        assert_eq!(verdict(&manifest, bmm101_full()), Support::DeclinedGeometry);
    }

    #[test]
    fn a_deck_fullscreen_only_widget_declines_bmm101() {
        let manifest = manifest_declaring(
            r#"[{ "type": "rectangular", "min_width": 1280, "max_width": 1280,
                  "min_height": 480, "max_height": 480 }]"#,
        );

        assert_eq!(verdict(&manifest, bmm101_full()), Support::DeclinedGeometry);
    }

    /// BMM101 runs at 165 DPI, so a density floor above it declines the size
    /// it otherwise declares.
    #[test]
    fn a_density_floor_declines_bmm101_on_dpi() {
        let manifest = manifest_declaring(
            r#"[{ "type": "rectangular", "min_width": 317, "max_width": 1280,
                  "min_height": 238, "max_height": 480, "min_dpi": 200 }]"#,
        );

        assert_eq!(verdict(&manifest, bmm101_full()), Support::DeclinedDpi);
    }

    #[test]
    fn naming_a_platform_sweeps_only_its_viewports() {
        let targets = sweep_targets(Some("bmm101")).expect("BUG: bmm101 must be in the catalog");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].to_string(), "bmm101:full");
    }

    #[test]
    fn the_whole_catalog_is_swept_when_no_platform_is_named() {
        let targets = sweep_targets(None).expect("BUG: the catalog sweep cannot fail");

        let expected: usize = PLATFORMS.iter().map(|p| p.viewports.len()).sum();
        assert_eq!(targets.len(), expected);
    }

    #[test]
    fn an_unknown_platform_is_an_error() {
        assert!(sweep_targets(Some("nope")).is_err());
    }
}
