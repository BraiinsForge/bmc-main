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

//! Capture configuration parsing for visual regression testing.
//!
//! Parses `capture/config.toml` from the widget crate root,
//! shared between the capture binary and testbed recording.
//! Every target it names resolves through [`crate::platform_catalog`],
//! so a config cannot describe a geometry the device does not have.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::platform_catalog::Target;

// ── Structured config errors ────────────────────────────────────────

/// A config error with enough structure for pretty-printing.
#[derive(Debug)]
pub struct ConfigError {
    pub path: PathBuf,
    pub message: String,
    pub hint: Option<String>,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConfigError {}

// ── Types ────────────────────────────────────────────────────────────

/// A named dataset and the targets it is replayed against.
///
/// A fixture carries no geometry of its own,
/// so one dataset can drive several targets — and one target, several datasets.
#[derive(Debug)]
pub struct FixtureEntry {
    /// Path to the `.jsonl.gz` recording, relative to the config directory
    /// until [`load_from_capture_dir`] resolves it.
    pub path: PathBuf,
    pub targets: Vec<Target>,
    /// KV seed applied on top of the fixture's own.
    pub kv: HashMap<String, String>,
    /// Overrides the config-wide `settle_delay` for this dataset.
    pub settle_delay: Option<u32>,
}

#[derive(Debug, Default)]
pub struct CaptureConfig {
    pub settle_delay: u32,
    /// Replay at the widget's own frame cadence (`request_frame_after`)
    /// instead of force-rendering every virtual frame.
    ///
    /// A widget that decouples its data fold from the render loop folds
    /// on the coalesced schedule the device host uses, so replay samples
    /// the state hardware would — needed for the fleet widget, whose fold
    /// is gated on a ~1s interval.
    ///
    /// Off by default, so every other widget keeps its per-frame baselines.
    pub honor_frame_schedule: bool,
    /// Datasets by name. The name is the recorded file's stem, the config key,
    /// and the last path component of the frames it produces.
    pub fixtures: BTreeMap<String, FixtureEntry>,
    /// Directory containing `config.toml` (set by [`load_from_capture_dir`],
    /// `None` when parsed from a bare string). Used to resolve relative
    /// fixture paths.
    pub config_dir: Option<PathBuf>,
}

impl CaptureConfig {
    /// Every (dataset, target) pair to capture,
    /// ordered by dataset name so a run's output is reproducible.
    #[must_use]
    pub fn capture_matrix(&self) -> Vec<(&str, Target)> {
        self.fixtures
            .iter()
            .flat_map(|(name, entry)| {
                entry
                    .targets
                    .iter()
                    .map(move |target| (name.as_str(), *target))
            })
            .collect()
    }

    /// Frames to let a dataset settle after its I/O drains, before the shot.
    #[must_use]
    pub fn settle_delay_for(&self, dataset: &str) -> u32 {
        self.fixtures
            .get(dataset)
            .and_then(|entry| entry.settle_delay)
            .unwrap_or(self.settle_delay)
    }

    /// Where a (dataset, target) pair's frames live, under an output root.
    #[must_use]
    pub fn frame_dir(root: &Path, dataset: &str, target: Target) -> PathBuf {
        root.join(target.platform.id)
            .join(target.viewport.id)
            .join(dataset)
    }
}

// ── Config loading ───────────────────────────────────────────────────

/// Load config directly from a `capture/` directory.
///
/// Returns a default config if `config.toml` doesn't exist in the directory.
pub fn load_from_capture_dir(capture_dir: &Path) -> Result<CaptureConfig> {
    if let Some(config) = try_load_from_dir(capture_dir)? {
        return Ok(config);
    }
    Ok(CaptureConfig::default())
}

/// Try to load and parse `config.toml` from a capture directory.
fn try_load_from_dir(capture_dir: &Path) -> Result<Option<CaptureConfig>> {
    let candidate = capture_dir.join("config.toml");
    if !candidate.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&candidate)
        .with_context(|| format!("failed to read {}", candidate.display()))?;
    let mut config = parse_capture_config(&content).map_err(|e| ConfigError {
        path: candidate.clone(),
        // `{e:#}`, so the cause under the section name survives.
        message: format!("{e:#}"),
        hint: Some(format!("valid keys: {}", KNOWN_CONFIG_KEYS.join(", "))),
    })?;
    // Resolve relative fixture paths against the config directory.
    config.config_dir = Some(capture_dir.to_owned());
    for (dataset, entry) in &mut config.fixtures {
        if entry.path.is_relative() {
            entry.path = capture_dir.join(&entry.path);
        }
        if !entry.path.exists() {
            return Err(ConfigError {
                path: candidate.clone(),
                message: format!("fixture '{dataset}' not found: {}", entry.path.display()),
                hint: Some(format!(
                    "record one with: just wasm::record <widget> {} {dataset}",
                    entry
                        .targets
                        .first()
                        .map_or_else(|| "<platform>:<viewport>".to_owned(), Target::to_string)
                )),
            }
            .into());
        }
    }
    Ok(Some(config))
}

// ── Config parsing ───────────────────────────────────────────────────

/// All known top-level keys in capture/config.toml.
const KNOWN_CONFIG_KEYS: &[&str] = &["settle_delay", "honor_frame_schedule", "fixtures"];

/// All known keys inside a `[fixtures.<name>]` table.
const KNOWN_FIXTURE_KEYS: &[&str] = &["path", "targets", "kv", "settle_delay"];

pub fn parse_capture_config(content: &str) -> Result<CaptureConfig> {
    let table: toml::Table = content.parse().context("capture.toml is not valid TOML")?;

    reject_unknown_keys(table.keys(), KNOWN_CONFIG_KEYS, "")?;

    Ok(CaptureConfig {
        settle_delay: parse_optional_u32(&table, "settle_delay")?.unwrap_or(0),
        honor_frame_schedule: parse_optional_bool(&table, "honor_frame_schedule")?.unwrap_or(false),
        fixtures: parse_fixtures_table(&table)?,
        config_dir: None,
    })
}

fn reject_unknown_keys<'a>(
    keys: impl Iterator<Item = &'a String>,
    known: &[&str],
    scope: &str,
) -> Result<()> {
    let unknown: Vec<String> = keys
        .filter(|k| !known.contains(&k.as_str()))
        .map(|k| format!("'{scope}{k}'"))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "unknown key(s): {} — valid keys: {}",
            unknown.join(", "),
            known.join(", "),
        );
    }
    Ok(())
}

// ── [fixtures.<name>] table parsing ──────────────────────────────────

fn parse_fixtures_table(table: &toml::Table) -> Result<BTreeMap<String, FixtureEntry>> {
    let Some(value) = table.get("fixtures") else {
        return Ok(BTreeMap::new());
    };
    let toml::Value::Table(fixtures) = value else {
        bail!("'[fixtures]' must be a table of named datasets");
    };

    let mut map = BTreeMap::new();
    for (dataset, value) in fixtures {
        let toml::Value::Table(entry) = value else {
            bail!(
                "'[fixtures.{dataset}]' must be a table with 'path' and 'targets' \
                 (a bare path is the retired size-keyed form)"
            );
        };
        map.insert(
            dataset.clone(),
            parse_fixture_entry(dataset, entry).with_context(|| format!("fixtures.{dataset}"))?,
        );
    }
    Ok(map)
}

/// What a target's dataset is called when nothing else names it.
///
/// The convention the corpus follows, so re-recording lands on the dataset
/// a target already has rather than minting a sibling.
#[must_use]
pub fn conventional_dataset_name(target: Target) -> String {
    format!("{}-{}", target.platform.id, target.viewport.id)
}

/// Whether a dataset name is safe to use as one.
///
/// The name becomes a file name, a config key and an output directory,
/// so anything path-like would put frames or fixtures somewhere unintended.
#[must_use]
pub fn is_valid_dataset_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name != "."
        && name != ".."
}

fn parse_fixture_entry(dataset: &str, entry: &toml::Table) -> Result<FixtureEntry> {
    if !is_valid_dataset_name(dataset) {
        bail!("dataset name must be non-empty and use only letters, digits, '-', '_' or '.'");
    }

    reject_unknown_keys(entry.keys(), KNOWN_FIXTURE_KEYS, &format!("{dataset}."))?;

    let path = entry
        .get("path")
        .and_then(toml::Value::as_str)
        .context("'path' is required and must be a string")?;

    let target_names = parse_string_array(entry, "targets")?;
    if target_names.is_empty() {
        bail!("'targets' is required and must list at least one <platform>:<viewport>");
    }
    let targets = target_names
        .iter()
        .map(|name| name.parse::<Target>().map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;

    let kv = match entry.get("kv") {
        Some(toml::Value::Table(t)) => parse_kv_table(t, "kv")?,
        Some(_) => bail!("'kv' must be a table of string key-value pairs"),
        None => HashMap::new(),
    };

    Ok(FixtureEntry {
        path: PathBuf::from(path),
        targets,
        kv,
        settle_delay: parse_optional_u32(entry, "settle_delay")?,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn parse_optional_u32(table: &toml::Table, key: &str) -> Result<Option<u32>> {
    match table.get(key) {
        Some(toml::Value::Integer(n)) => {
            let n = *n;
            u32::try_from(n)
                .map(Some)
                .with_context(|| format!("'{key}' must be a non-negative integer, got {n}"))
        }
        Some(_) => bail!("'{key}' must be an integer"),
        None => Ok(None),
    }
}

fn parse_optional_bool(table: &toml::Table, key: &str) -> Result<Option<bool>> {
    match table.get(key) {
        Some(toml::Value::Boolean(b)) => Ok(Some(*b)),
        Some(_) => bail!("'{key}' must be a boolean"),
        None => Ok(None),
    }
}

fn parse_string_array(table: &toml::Table, key: &str) -> Result<Vec<String>> {
    match table.get(key) {
        Some(toml::Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for (i, v) in a.iter().enumerate() {
                out.push(
                    v.as_str()
                        .with_context(|| format!("{key}[{i}] must be a string"))?
                        .to_owned(),
                );
            }
            Ok(out)
        }
        Some(_) => bail!("'{key}' must be an array of strings"),
        None => Ok(Vec::new()),
    }
}

fn parse_kv_table(t: &toml::Table, ctx: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::with_capacity(t.len());
    for (k, v) in t {
        let s = v
            .as_str()
            .with_context(|| format!("{ctx}.{k} must be a string value"))?;
        map.insert(k.clone(), s.to_owned());
    }
    Ok(map)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog offers this name on a button,
    /// so an id the validator refuses would fill the field with a rejection.
    #[test]
    fn every_target_has_a_conventional_name_it_could_be_saved_under() {
        for platform in crate::platform_catalog::PLATFORMS {
            for viewport in platform.viewports {
                let target = Target { platform, viewport };
                let name = conventional_dataset_name(target);
                assert!(
                    is_valid_dataset_name(&name),
                    "{target} would be offered '{name}', which is not a usable dataset name",
                );
            }
        }
    }

    // ── Config validation ────────────────────────────────────────────

    #[test]
    fn config_unknown_key_rejected() {
        let toml = r"
            settle_delay = 5
            typo_key = 42
        ";
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(msg.contains("typo_key"), "should name the bad key: {msg}");
    }

    #[test]
    fn config_multiple_unknown_keys() {
        let toml = r"
            settl_delay = 5
            iteractions = []
        ";
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("settl_delay") && msg.contains("iteractions"),
            "should list all bad keys: {msg}"
        );
    }

    #[test]
    fn config_all_known_keys_accepted() {
        let toml = r#"
            settle_delay = 5
            honor_frame_schedule = true

            [fixtures.mining]
            path = "fixtures/mining.jsonl.gz"
            targets = ["bmm100:full"]
            settle_delay = 40
            kv = { theme = "dark" }
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: all known keys should be accepted");
        assert_eq!(cfg.settle_delay_for("mining"), 40);
        assert_eq!(cfg.fixtures["mining"].kv["theme"], "dark");
    }

    #[test]
    fn retired_keys_are_rejected() {
        for retired in [
            "timeout = 100",
            "record_timeout = 30",
            r#"sizes = ["full"]"#,
            "[kv]\ntheme = \"dark\"",
            "[[variants]]\nname = \"dark\"",
        ] {
            assert!(
                parse_capture_config(retired).is_err(),
                "retired key must be rejected: {retired}"
            );
        }
    }

    #[test]
    fn config_empty_is_valid() {
        let cfg = parse_capture_config("").expect("BUG: empty config should be valid");
        assert_eq!(cfg.settle_delay, 0);
        assert!(cfg.fixtures.is_empty());
        assert!(!cfg.honor_frame_schedule);
    }

    #[test]
    fn config_honor_frame_schedule_parsed() {
        let cfg = parse_capture_config("honor_frame_schedule = true")
            .expect("BUG: honor_frame_schedule config should parse");
        assert!(cfg.honor_frame_schedule);
    }

    #[test]
    fn config_honor_frame_schedule_rejects_non_bool() {
        let err = parse_capture_config("honor_frame_schedule = 1")
            .expect_err("BUG: non-boolean honor_frame_schedule must fail to parse");
        assert!(
            format!("{err:#}").contains("honor_frame_schedule"),
            "should name the bad key: {err:#}"
        );
    }

    #[test]
    fn config_time_field_rejected() {
        let toml = r#"
            time = "2026-03-10T18:00:00"
            settle_delay = 10
        "#;
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("time"),
            "should reject 'time' as unknown: {msg}"
        );
    }

    #[test]
    fn settle_delay_falls_back_to_the_config_wide_value() {
        let toml = r#"
            settle_delay = 5

            [fixtures.common]
            path = "fixtures/common.jsonl.gz"
            targets = ["bmc100:full"]
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: config should parse");
        assert_eq!(cfg.settle_delay_for("common"), 5);
        assert_eq!(
            cfg.settle_delay_for("no-such-dataset"),
            5,
            "an unknown dataset still reports the config-wide default"
        );
    }

    // ── [fixtures] table ─────────────────────────────────────────────

    #[test]
    fn one_dataset_binds_to_many_targets() {
        let toml = r#"
            [fixtures.common]
            path = "fixtures/common.jsonl.gz"
            targets = ["bmc100:full", "bmc100:large", "bmc100:medium", "bmc100:small"]
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: multi-target config should parse");
        let matrix = cfg.capture_matrix();
        assert_eq!(matrix.len(), 4);
        assert!(matrix.iter().all(|(dataset, _)| *dataset == "common"));
        assert_eq!(matrix[0].1.to_string(), "bmc100:full");
    }

    #[test]
    fn one_target_takes_many_datasets() {
        let toml = r#"
            [fixtures.mining]
            path = "fixtures/mining.jsonl.gz"
            targets = ["bfm100:full"]

            [fixtures.idle]
            path = "fixtures/idle.jsonl.gz"
            targets = ["bfm100:full"]
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: multi-dataset config should parse");
        let matrix = cfg.capture_matrix();
        assert_eq!(matrix.len(), 2);
        assert_eq!(
            matrix.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            ["idle", "mining"],
            "datasets iterate in name order so a run is reproducible",
        );
    }

    #[test]
    fn frames_land_under_platform_viewport_dataset() {
        let target: Target = "bmc100:small".parse().expect("BUG: target must parse");
        assert_eq!(
            CaptureConfig::frame_dir(Path::new("out"), "common", target),
            PathBuf::from("out/bmc100/small/common"),
        );
    }

    #[test]
    fn fixture_entry_requires_path_and_targets() {
        let missing_targets = r#"
            [fixtures.mining]
            path = "fixtures/mining.jsonl.gz"
        "#;
        let err = parse_capture_config(missing_targets)
            .expect_err("BUG: a dataset without targets must fail");
        assert!(format!("{err:#}").contains("targets"), "{err:#}");

        let missing_path = r#"
            [fixtures.mining]
            targets = ["bmc100:full"]
        "#;
        let err = parse_capture_config(missing_path)
            .expect_err("BUG: a dataset without a path must fail");
        assert!(format!("{err:#}").contains("path"), "{err:#}");

        let empty_targets = r#"
            [fixtures.mining]
            path = "fixtures/mining.jsonl.gz"
            targets = []
        "#;
        let err =
            parse_capture_config(empty_targets).expect_err("BUG: an empty targets list must fail");
        assert!(format!("{err:#}").contains("targets"), "{err:#}");
    }

    #[test]
    fn unknown_targets_name_what_is_available() {
        for (toml, expected) in [
            (
                r#"
                [fixtures.mining]
                path = "f.jsonl.gz"
                targets = ["nope:full"]
                "#,
                "bmc100",
            ),
            (
                r#"
                [fixtures.mining]
                path = "f.jsonl.gz"
                targets = ["bmm100:small"]
                "#,
                "full",
            ),
            (
                r#"
                [fixtures.mining]
                path = "f.jsonl.gz"
                targets = ["bmc100"]
                "#,
                "<platform>:<viewport>",
            ),
        ] {
            let err = parse_capture_config(toml).expect_err("BUG: a bad target must fail to parse");
            assert!(format!("{err:#}").contains(expected), "{err:#}");
        }
    }

    #[test]
    fn the_retired_size_keyed_form_is_rejected_with_a_hint() {
        let toml = r#"
            [fixtures]
            full = "fixtures/full.jsonl.gz"
        "#;
        let err = parse_capture_config(toml)
            .expect_err("BUG: the size-keyed fixture form must fail to parse");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("targets") && msg.contains("size-keyed"),
            "{msg}"
        );
    }

    #[test]
    fn a_path_like_dataset_name_is_rejected() {
        for bad in ["..", "../escape", "a/b", "with space", "sub/dir"] {
            let toml = format!(
                r#"
                [fixtures."{bad}"]
                path = "f.jsonl.gz"
                targets = ["bmc100:full"]
                "#
            );
            assert!(
                parse_capture_config(&toml).is_err(),
                "dataset name '{bad}' becomes a path component and must be rejected",
            );
        }
    }

    #[test]
    fn unknown_fixture_key_is_rejected() {
        let toml = r#"
            [fixtures.mining]
            path = "f.jsonl.gz"
            targets = ["bmc100:full"]
            sizes = ["full"]
        "#;
        let err = parse_capture_config(toml).expect_err("BUG: unknown fixture key must fail");
        assert!(format!("{err:#}").contains("sizes"), "{err:#}");
    }

    #[test]
    fn parse_fixtures_wrong_type_rejected() {
        let toml = r#"
            fixtures = "not a table"
        "#;
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(msg.contains("[fixtures]"), "{msg}");
    }

    #[test]
    fn a_round_target_keeps_its_shape_through_the_config() {
        let toml = r#"
            [fixtures.round]
            path = "fixtures/round.jsonl.gz"
            targets = ["bfm100:full"]
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: round config should parse");
        let (_, target) = cfg.capture_matrix()[0];
        assert_eq!(
            target.viewport.shape,
            crate::platform_catalog::DisplayShape::Round
        );
        assert_eq!((target.viewport.width, target.viewport.height), (480, 480));
    }
}
