// Copyright (C) 2026  Braiins Systems s.r.o.

//! Capture configuration parsing for visual regression testing.
//!
//! Shared between the capture binary and (in the future) testbed recording.
//! Parses `capture/config.toml` from the widget crate root.
//!
//! Capture keeps its own `CAPTURE_SIZES` and does not yet consume the testbed
//! platform catalog. Unifying capture sizes with the platform catalog is a
//! separate later task.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

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

// ── Constants ────────────────────────────────────────────────────────

/// Standard widget capture sizes: (name, width, height).
pub const CAPTURE_SIZES: &[(&str, u32, u32)] = &[
    ("full", 1280, 480),
    ("large", 638, 480),
    ("medium", 638, 238),
    ("small", 317, 238),
    ("round", 480, 480),
    ("bmm101", 480, 320),
];

/// Valid size names for per-size fixture/interaction blocks.
pub const VALID_SIZES: &[&str] = &["full", "large", "medium", "small", "round", "bmm101"];

/// Look up a size name from pixel dimensions.
#[must_use]
pub fn size_name_from_dimensions(width: u32, height: u32) -> &'static str {
    CAPTURE_SIZES
        .iter()
        .find(|(_, w, h)| *w == width && *h == height)
        .map_or("custom", |(name, _, _)| name)
}

/// Format a size as "WxH" from its name. Returns `None` for unknown names.
#[must_use]
pub fn size_dimensions_str(name: &str) -> Option<String> {
    CAPTURE_SIZES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, w, h)| format!("{w}x{h}"))
}

/// Default settlement timeout in frames (~5s virtual time).
pub const DEFAULT_TIMEOUT: u32 = 300;

/// Hard wall-clock cap for recording mode (seconds).
pub const RECORD_WALL_CAP_SECS: f64 = 120.0;

// ── Types ────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CaptureConfig {
    pub settle_delay: u32,
    /// Explicit list of sizes to capture (empty = all sizes).
    pub sizes: Vec<String>,
    pub timeout: u32,
    /// Wall-clock cap for recording mode (seconds). Default: 120.
    pub record_timeout: f64,
    /// Default KV values applied to all variants.
    pub kv: HashMap<String, String>,
    /// Named KV variants (each merges on top of `kv`).
    pub variants: Vec<CaptureVariant>,
    /// Per-size unified fixture file paths (relative to `config_dir`).
    /// Key = size name, value = path to `.json` fixture file.
    pub fixtures: HashMap<String, PathBuf>,
    /// Directory containing `config.toml` (set by [`load_from_capture_dir`],
    /// `None` when parsed from a bare string). Used to resolve relative
    /// fixture paths.
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub struct CaptureVariant {
    pub name: String,
    pub kv: HashMap<String, String>,
}

// ── Config loading ───────────────────────────────────────────────────

/// Load config directly from a `capture/` directory.
///
/// Returns a default config if `config.toml` doesn't exist in the directory.
pub fn load_from_capture_dir(capture_dir: &Path) -> Result<CaptureConfig> {
    if let Some(config) = try_load_from_dir(capture_dir)? {
        return Ok(config);
    }
    Ok(CaptureConfig {
        timeout: DEFAULT_TIMEOUT,
        record_timeout: RECORD_WALL_CAP_SECS,
        ..Default::default()
    })
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
        message: format!("{e}"),
        hint: Some(format!("valid keys: {}", KNOWN_CONFIG_KEYS.join(", "))),
    })?;
    // Resolve relative fixture paths against the config directory.
    config.config_dir = Some(capture_dir.to_owned());
    for (size, path) in &mut config.fixtures {
        if path.is_relative() {
            *path = capture_dir.join(&*path);
        }
        if !path.exists() {
            return Err(ConfigError {
                path: candidate.clone(),
                message: format!("fixture for size '{size}' not found: {}", path.display()),
                hint: Some("record one with: make record EXAMPLE=<name> SIZE=<size>".into()),
            }
            .into());
        }
    }
    Ok(Some(config))
}

// ── Config parsing ───────────────────────────────────────────────────

/// All known top-level keys in capture/config.toml.
const KNOWN_CONFIG_KEYS: &[&str] = &[
    "settle_delay",
    "timeout",
    "record_timeout",
    "sizes",
    "kv",
    "variants",
    "fixtures",
];

#[expect(clippy::cast_precision_loss)]
pub fn parse_capture_config(content: &str) -> Result<CaptureConfig> {
    let table: toml::Table = content.parse().context("capture.toml is not valid TOML")?;

    // Reject unknown keys early so typos don't silently vanish.
    let unknown: Vec<&String> = table
        .keys()
        .filter(|k| !KNOWN_CONFIG_KEYS.contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "unknown key(s): {}",
            unknown
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    let settle_delay = parse_optional_u32(&table, "settle_delay")?.unwrap_or(0);
    let timeout = parse_optional_u32(&table, "timeout")?.unwrap_or(DEFAULT_TIMEOUT);

    let record_timeout = match table.get("record_timeout") {
        Some(toml::Value::Float(f)) => *f,
        Some(toml::Value::Integer(n)) => *n as f64,
        Some(_) => bail!("'record_timeout' must be a number (seconds)"),
        None => RECORD_WALL_CAP_SECS,
    };

    let sizes = parse_string_array(&table, "sizes")?;

    let kv = match table.get("kv") {
        Some(toml::Value::Table(t)) => parse_kv_table(t, "kv")?,
        Some(_) => bail!("'[kv]' must be a table of string key-value pairs"),
        None => HashMap::new(),
    };

    let variants = parse_variants(&table)?;

    let fixtures = parse_fixtures_table(&table)?;

    Ok(CaptureConfig {
        settle_delay,
        sizes,
        timeout,
        record_timeout,
        kv,
        variants,
        fixtures,
        config_dir: None,
    })
}

// ── [fixtures] table parsing ─────────────────────────────────────────

fn parse_fixtures_table(table: &toml::Table) -> Result<HashMap<String, PathBuf>> {
    let Some(toml::Value::Table(t)) = table.get("fixtures") else {
        // No [fixtures] section or wrong type — that's fine for legacy configs.
        if table.get("fixtures").is_some_and(|v| !v.is_table()) {
            bail!("'[fixtures]' must be a table mapping size names to fixture file paths");
        }
        return Ok(HashMap::new());
    };

    let mut map = HashMap::with_capacity(t.len());
    for (key, val) in t {
        if !VALID_SIZES.contains(&key.as_str()) {
            bail!("fixtures: unknown size '{key}' — valid sizes: {VALID_SIZES:?}");
        }
        let path = val
            .as_str()
            .with_context(|| format!("fixtures.{key} must be a string path"))?;
        map.insert(key.clone(), PathBuf::from(path));
    }
    Ok(map)
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

fn parse_variants(table: &toml::Table) -> Result<Vec<CaptureVariant>> {
    match table.get("variants") {
        Some(toml::Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                let t = v
                    .as_table()
                    .with_context(|| format!("[[variants]][{i}] must be a table"))?;
                let name = t
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .with_context(|| format!("[[variants]][{i}] must have a 'name' string"))?
                    .to_owned();
                let variant_kv = match t.get("kv") {
                    Some(toml::Value::Table(kt)) => {
                        parse_kv_table(kt, &format!("variants[{i}].kv"))?
                    }
                    Some(_) => {
                        bail!("variants[{i}].kv must be a table of string key-value pairs")
                    }
                    None => HashMap::new(),
                };
                out.push(CaptureVariant {
                    name,
                    kv: variant_kv,
                });
            }
            Ok(out)
        }
        Some(_) => bail!("'variants' must be an array of tables (use [[variants]])"),
        None => Ok(Vec::new()),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
            timeout = 100
            record_timeout = 30
            sizes = ["480x480"]

            [kv]
            theme = "dark"
        "#;
        parse_capture_config(toml).expect("BUG: all known keys should be accepted");
    }

    #[test]
    fn config_empty_is_valid() {
        let cfg = parse_capture_config("").expect("BUG: empty config should be valid");
        assert_eq!(cfg.settle_delay, 0);
        assert!(cfg.fixtures.is_empty());
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
    fn config_sizes_parsed() {
        let toml = r#"
            sizes = ["full", "small"]
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: sizes config should parse");
        assert_eq!(cfg.sizes, vec!["full", "small"]);
    }

    #[test]
    fn config_kv_parsed() {
        let toml = r#"
            [kv]
            theme = "dark"
            lang = "en"
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: kv config should parse");
        assert_eq!(cfg.kv["theme"], "dark");
        assert_eq!(cfg.kv["lang"], "en");
    }

    #[test]
    fn config_variants_parsed() {
        let toml = r#"
            [[variants]]
            name = "dark"

            [variants.kv]
            theme = "dark"

            [[variants]]
            name = "light"

            [variants.kv]
            theme = "light"
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: variants config should parse");
        assert_eq!(cfg.variants.len(), 2);
        assert_eq!(cfg.variants[0].name, "dark");
        assert_eq!(cfg.variants[0].kv["theme"], "dark");
        assert_eq!(cfg.variants[1].name, "light");
    }

    // ── [fixtures] table ─────────────────────────────────────────────

    #[test]
    fn parse_fixtures_table_valid() {
        let toml = r#"
            [fixtures]
            full = "fixtures/full.json"
            large = "fixtures/full.json"
            small = "fixtures/small.json"
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: fixtures config should parse");
        assert_eq!(cfg.fixtures.len(), 3);
        assert_eq!(cfg.fixtures["full"], PathBuf::from("fixtures/full.json"));
        assert_eq!(cfg.fixtures["small"], PathBuf::from("fixtures/small.json"));
    }

    #[test]
    fn parse_fixtures_invalid_size_rejected() {
        let toml = r#"
            [fixtures]
            huge = "fixtures/huge.json"
        "#;
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown size 'huge'"), "{msg}");
    }

    #[test]
    fn parse_fixtures_non_string_rejected() {
        let toml = r"
            [fixtures]
            full = 42
        ";
        let err =
            parse_capture_config(toml).expect_err("BUG: invalid capture config must fail to parse");
        let msg = format!("{err:#}");
        assert!(msg.contains("fixtures.full"), "{msg}");
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
    fn config_with_only_fixtures_accepted() {
        let toml = r#"
            settle_delay = 10

            [fixtures]
            full = "fixtures/full.json"
            large = "fixtures/large.json"
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: fixtures-only config should parse");
        assert_eq!(cfg.fixtures.len(), 2);
    }

    #[test]
    fn round_size_name_is_recognized_from_480_square() {
        assert_eq!(size_name_from_dimensions(480, 480), "round");
        assert_eq!(size_dimensions_str("round").as_deref(), Some("480x480"));
    }

    #[test]
    fn bmm101_size_name_is_recognized_from_480x320() {
        assert_eq!(size_name_from_dimensions(480, 320), "bmm101");
        assert_eq!(size_dimensions_str("bmm101").as_deref(), Some("480x320"));
    }

    #[test]
    fn config_accepts_round_size_and_fixture() {
        let toml = r#"
            sizes = ["round"]

            [fixtures]
            round = "fixtures/round.jsonl.gz"
        "#;
        let cfg = parse_capture_config(toml).expect("BUG: round config should parse");
        assert_eq!(cfg.sizes, vec!["round"]);
        assert_eq!(
            cfg.fixtures["round"],
            PathBuf::from("fixtures/round.jsonl.gz")
        );
    }
}
