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

//! `intl bump-icu4x` and `intl bump-formatjs`: an upstream release, with everything that moves in step.
//!
//! Every upstream lookup, and every check of the files about to be written, runs first,
//! so a failure there leaves the checkout as it was;
//! a step that fails after the writes begin leaves them for `git diff` to show.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::formatjs::CLI_PIN;
use crate::{console, nix_pin};

/// Every ICU4X crate the workspace declares is required by tilde:
/// its minor is the one datagen and the CLDR pin are held to.
/// Each ICU4X crate requires `icu_provider` on the minor it ships,
/// so `icu_provider`'s tilde holds the undeclared ones too, once `cargo update` names them.
const ICU4X_PREFIX: &str = "icu";

/// Required exactly: the macros hash host message ids as the CLI does.
const FORMATJS_DECLARED: [&str; 4] = [
    "formatjs_icu_messageformat",
    "formatjs_icu_messageformat_parser",
    "formatjs_intl",
    "formatjs_intl_macros",
];

const DATAGEN_PIN: &str = "nix/intl/icu4x-datagen.nix";
const CLDR_PIN: &str = "nix/intl/cldr-json.nix";

/// Nix's `lib.fakeHash`: a build against it fails, naming the hash it should have been.
const FAKE_HASH: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// # Errors
/// An upstream lookup or a pin that fails, or a step that fails once the writes began.
pub fn icu4x(minor: &str, repo: &Path) -> Result<ExitCode, String> {
    let target = parse_version(&format!("{minor}.0"))?;
    let cargo_toml = repo.join("Cargo.toml");
    let mut requirements = BTreeMap::new();
    for name in declared_icu4x(&cargo_toml)? {
        let newest = newest_in_minor(&name, &crate_index(&name)?, &target)?;
        requirements.insert(name, format!("~{newest}"));
    }
    let datagen = newest_in_minor("icu4x-datagen", &crate_index("icu4x-datagen")?, &target)?;
    let (datagen_hash, datagen_tree) = prefetch(&crate_url("icu4x-datagen", &datagen))?;
    // The source crate datagen builds with, which is not always the newest of the minor.
    let source = locked_version(&datagen_tree.join("Cargo.lock"), "icu_provider_source")?;
    let (_, source_tree) = prefetch(&crate_url("icu_provider_source", &source))?;
    let lib_rs = source_tree.join("src/lib.rs");
    let cldr_tag = cldr_tag_of(
        &std::fs::read_to_string(&lib_rs).map_err(|error| console::at(&lib_rs, error))?,
    )?;
    let (cldr_hash, _) = prefetch(&format!(
        "https://github.com/unicode-org/cldr-json/releases/download/{cldr_tag}/cldr-{cldr_tag}-json-full.zip"
    ))?;
    let datagen_pin = repo.join(DATAGEN_PIN);
    let cldr_pin = repo.join(CLDR_PIN);
    for (pin, name) in [
        (&datagen_pin, "version"),
        (&datagen_pin, "hash"),
        (&datagen_pin, "cargoHash"),
        (&cldr_pin, "tag"),
        (&cldr_pin, "hash"),
    ] {
        nix_pin::get(pin, name)?;
    }

    set_requirements(&cargo_toml, &requirements)?;
    cargo_update(repo, &locked_icu4x(&repo.join("Cargo.lock"))?)?;
    nix_pin::set(&datagen_pin, "version", &datagen.to_string())?;
    nix_pin::set(&datagen_pin, "hash", &datagen_hash)?;
    let cargo_hash = cargo_hash(repo, "icu4x-datagen", &datagen_pin)?;
    nix_pin::set(&datagen_pin, "cargoHash", &cargo_hash)?;
    nix_pin::set(&cldr_pin, "tag", &cldr_tag)?;
    nix_pin::set(&cldr_pin, "hash", &cldr_hash)?;
    console::ok(&format!(
        "ICU4X {minor}: icu4x-datagen {datagen}, CLDR {cldr_tag}"
    ));
    Ok(ExitCode::SUCCESS)
}

/// # Errors
/// An upstream lookup or a pin that fails, or a step that fails once the writes began.
pub fn formatjs(version: &str, repo: &Path) -> Result<ExitCode, String> {
    let target = parse_version(version)?;
    let (source_hash, tree) = prefetch(&format!(
        "https://github.com/formatjs/formatjs/archive/refs/tags/formatjs_cli_v{target}.tar.gz"
    ))?;
    let released = workspace_versions(&tree)?;
    let shipped = released.get("formatjs_cli").map(String::as_str);
    if shipped != Some(target.to_string().as_str()) {
        return Err(format!(
            "formatjs_cli_v{target} ships formatjs_cli {}",
            shipped.unwrap_or("not at all")
        ));
    }
    let missing: Vec<&str> = FORMATJS_DECLARED
        .into_iter()
        .filter(|name| !released.contains_key(*name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "formatjs_cli_v{target} ships no {}",
            missing.join(", ")
        ));
    }
    let cli_pin = repo.join(CLI_PIN);
    for name in ["version", "hash", "cargoHash"] {
        nix_pin::get(&cli_pin, name)?;
    }

    let requirements = FORMATJS_DECLARED
        .into_iter()
        .map(|name| (name.to_owned(), format!("={}", released[name])))
        .collect();
    set_requirements(&repo.join("Cargo.toml"), &requirements)?;
    cargo_update(repo, &FORMATJS_DECLARED.map(str::to_owned))?;
    nix_pin::set(&cli_pin, "version", &target.to_string())?;
    nix_pin::set(&cli_pin, "hash", &source_hash)?;
    let cargo_hash = cargo_hash(repo, "formatjs-cli", &cli_pin)?;
    nix_pin::set(&cli_pin, "cargoHash", &cargo_hash)?;
    console::ok(&format!("FormatJS {target}"));
    Ok(ExitCode::SUCCESS)
}

fn parse_version(text: &str) -> Result<Version, String> {
    Version::parse(text)
        .map_err(|error| format!("{}: not a version: {error}", console::refused(text)))
}

/// The newest stable, unyanked release of `minor` in `name`'s sparse-index lines.
fn newest_in_minor(name: &str, index: &str, minor: &Version) -> Result<Version, String> {
    #[derive(Deserialize)]
    struct Release {
        vers: String,
        yanked: bool,
    }
    let mut newest: Option<Version> = None;
    for line in index.lines().filter(|line| !line.is_empty()) {
        let release: Release = serde_json::from_str(line)
            .map_err(|error| format!("the crates.io index of {name} does not read: {error}"))?;
        if release.yanked {
            continue;
        }
        let version = parse_version(&release.vers)?;
        if (version.major, version.minor) == (minor.major, minor.minor)
            && version.pre.is_empty()
            && newest.as_ref().is_none_or(|newest| version > *newest)
        {
            newest = Some(version);
        }
    }
    newest.ok_or_else(|| {
        format!(
            "{name} has no stable release of {}.{}",
            minor.major, minor.minor
        )
    })
}

/// `TESTED_CLDR_TAG` of `icu_provider_source`: the CLDR release it is tested against.
fn cldr_tag_of(lib_rs: &str) -> Result<String, String> {
    lib_rs
        .split_once("TESTED_CLDR_TAG")
        .and_then(|(_, rest)| rest.split_once('=')?.1.trim_start().strip_prefix('"'))
        .and_then(|rest| rest.split_once('"'))
        .map(|(tag, _)| tag.to_owned())
        .ok_or_else(|| "icu_provider_source declares no TESTED_CLDR_TAG".to_owned())
}

/// The version of every crate under `crates/` in a FormatJS release tree, by name,
/// one inherited from the workspace resolved to the workspace's.
fn workspace_versions(tree: &Path) -> Result<BTreeMap<String, String>, String> {
    #[derive(Deserialize)]
    struct Root {
        workspace: RootWorkspace,
    }
    #[derive(Deserialize)]
    struct RootWorkspace {
        package: Option<RootPackage>,
    }
    #[derive(Deserialize)]
    struct RootPackage {
        version: Option<String>,
    }
    #[derive(Deserialize)]
    struct Manifest {
        package: Package,
    }
    #[derive(Deserialize)]
    struct Package {
        name: String,
        version: PackageVersion,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PackageVersion {
        Given(String),
        Inherited {},
    }

    let root: Root = read_toml(&tree.join("Cargo.toml"))?;
    let inherited = root.workspace.package.and_then(|package| package.version);
    let crates = tree.join("crates");
    let listing = std::fs::read_dir(&crates).map_err(|error| console::at(&crates, error))?;
    let mut manifests: Vec<PathBuf> = listing
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("Cargo.toml"))
        .filter(|manifest| manifest.is_file())
        .collect();
    manifests.sort();
    let mut versions = BTreeMap::new();
    for path in manifests {
        let Manifest { package } = read_toml(&path)?;
        let version = match package.version {
            PackageVersion::Given(version) => version,
            PackageVersion::Inherited {} => inherited.clone().ok_or_else(|| {
                format!(
                    "{} inherits a version its workspace does not declare",
                    console::path(&path)
                )
            })?,
        };
        versions.insert(package.name, version);
    }
    Ok(versions)
}

/// The ICU4X crates `[workspace.dependencies]` declares.
fn declared_icu4x(cargo_toml: &Path) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Manifest {
        workspace: Workspace,
    }
    #[derive(Deserialize)]
    struct Workspace {
        #[serde(default)]
        dependencies: BTreeMap<String, toml::Value>,
    }
    let manifest: Manifest = read_toml(cargo_toml)?;
    Ok(manifest
        .workspace
        .dependencies
        .into_keys()
        .filter(|name| name.starts_with(ICU4X_PREFIX))
        .collect())
}

/// Every crate `lockfile` resolves, by name, with each version it resolves.
fn locked(lockfile: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    #[derive(Deserialize)]
    struct Lock {
        package: Vec<Locked>,
    }
    #[derive(Deserialize)]
    struct Locked {
        name: String,
        version: String,
    }
    let lock: Lock = read_toml(lockfile)?;
    let mut locked: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for package in lock.package {
        locked
            .entry(package.name)
            .or_default()
            .push(package.version);
    }
    Ok(locked)
}

/// Every ICU4X crate `lockfile` resolves.
fn locked_icu4x(lockfile: &Path) -> Result<Vec<String>, String> {
    Ok(locked(lockfile)?
        .into_keys()
        .filter(|name| name.starts_with(ICU4X_PREFIX))
        .collect())
}

/// The one version of `name` that `lockfile` resolves.
fn locked_version(lockfile: &Path, name: &str) -> Result<Version, String> {
    let versions = locked(lockfile)?.remove(name).unwrap_or_default();
    match &versions[..] {
        [version] => parse_version(version),
        _ => Err(format!(
            "{} locks {name} {} times, not once",
            console::path(lockfile),
            versions.len()
        )),
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|error| console::at(path, error))?;
    toml::from_str(&text).map_err(|error| console::at(path, error))
}

/// Set `[workspace.dependencies]` requirements, every other byte of the file as it was.
fn set_requirements(
    cargo_toml: &Path,
    requirements: &BTreeMap<String, String>,
) -> Result<(), String> {
    let text =
        std::fs::read_to_string(cargo_toml).map_err(|error| console::at(cargo_toml, error))?;
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error| console::at(cargo_toml, error))?;
    let dependencies = document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| {
            format!(
                "{} declares no workspace dependencies",
                console::path(cargo_toml)
            )
        })?;
    for (name, requirement) in requirements {
        let declared = dependencies
            .get_mut(name)
            .ok_or_else(|| format!("Cargo.toml declares no {name}"))?;
        let version = if declared.is_str() {
            Some(declared)
        } else {
            // A table, inline or spelt in dotted keys: `name.version = …`.
            declared
                .as_table_like_mut()
                .and_then(|table| table.get_mut("version"))
        }
        .and_then(toml_edit::Item::as_value_mut)
        .ok_or_else(|| format!("Cargo.toml declares {name} with no version"))?;
        let decor = version.decor().clone();
        *version = requirement.as_str().into();
        *version.decor_mut() = decor;
    }
    std::fs::write(cargo_toml, document.to_string()).map_err(|error| console::at(cargo_toml, error))
}

fn cargo_update(repo: &Path, crates: &[String]) -> Result<(), String> {
    let status = Command::new("cargo")
        .arg("update")
        .args(crates.iter().flat_map(|name| ["--package", name]))
        .current_dir(repo)
        .status()
        .map_err(|error| console::spawn_failed("cargo", &error))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo update` failed for {}", crates.join(", ")))
    }
}

/// Where the sparse index files a crate, by the length of its lowercased name.
fn index_path(name: &str) -> String {
    let name = name.to_lowercase();
    let part = |skip, take| name.chars().skip(skip).take(take).collect::<String>();
    match name.chars().count() {
        length @ (1 | 2) => format!("{length}/{name}"),
        3 => format!("3/{}/{name}", part(0, 1)),
        _ => format!("{}/{}/{name}", part(0, 2), part(2, 2)),
    }
}

/// The crate's lines in the crates.io sparse index, one release each.
fn crate_index(name: &str) -> Result<String, String> {
    read_index(
        name,
        &format!("https://index.crates.io/{}", index_path(name)),
    )
}

fn read_index(name: &str, url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    agent
        .get(url)
        .call()
        .and_then(|mut response| response.body_mut().read_to_string())
        .map_err(|error| format!("cannot read {name} from the crates.io index: {error}"))
}

/// As `fetchCrate` fetches it.
fn crate_url(name: &str, version: &Version) -> String {
    format!("https://static.crates.io/crates/{name}/{name}-{version}.crate")
}

/// The unpacked download's hash, as the nix fetchers pin it, and where it was unpacked.
fn prefetch(url: &str) -> Result<(String, PathBuf), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Prefetched {
        hash: String,
        store_path: PathBuf,
    }
    let output = Command::new("nix")
        .args(["store", "prefetch-file", "--unpack", "--json", url])
        .output()
        .map_err(|error| console::spawn_failed("nix", &error))?;
    if !output.status.success() {
        return Err(format!(
            "cannot fetch {url}:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    serde_json::from_slice::<Prefetched>(&output.stdout)
        .map(|fetched| (fetched.hash, fetched.store_path))
        .map_err(|error| format!("`nix store prefetch-file` printed no fetch: {error}"))
}

/// The `cargoHash` the flake's `package` needs now, read off a build against [`FAKE_HASH`].
fn cargo_hash(repo: &Path, package: &str, pin_file: &Path) -> Result<String, String> {
    probe_cargo_hash(package, pin_file, || {
        Command::new("nix")
            .args(["build", "--no-link", &format!(".#{package}.cargoDeps")])
            .current_dir(repo)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stderr).into_owned())
            .map_err(|error| console::spawn_failed("nix", &error))
    })
}

/// `build` runs against [`FAKE_HASH`] and gives its stderr; the pin is as it was unless a hash comes of it.
fn probe_cargo_hash(
    package: &str,
    pin_file: &Path,
    build: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let pinned = nix_pin::get(pin_file, "cargoHash")?;
    nix_pin::set(pin_file, "cargoHash", FAKE_HASH)?;
    let found = build().and_then(|stderr| {
        vendor_hash(&stderr).map(str::to_owned).ok_or_else(|| {
            format!(
                "the {package} vendor build did not name its hash:\n{}",
                stderr.trim_end()
            )
        })
    });
    if found.is_err() {
        nix_pin::set(pin_file, "cargoHash", &pinned)?;
    }
    found
}

/// The vendor derivation's own hash mismatch, not that of another fetch failing first.
fn vendor_hash(stderr: &str) -> Option<&str> {
    stderr
        .split("fixed-output derivation '")
        .skip(1)
        .find_map(|mismatch| {
            let (derivation, rest) = mismatch.split_once('\'')?;
            if !derivation.contains("-vendor") {
                return None;
            }
            let (_, got) = rest.split_once("got:")?;
            got.split_whitespace()
                .next()
                .filter(|hash| hash.starts_with("sha256-"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(releases: &[(&str, bool)]) -> String {
        releases
            .iter()
            .map(|(version, yanked)| {
                serde_json::json!({ "vers": version, "yanked": yanked }).to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn minor(text: &str) -> Version {
        parse_version(&format!("{text}.0")).expect("BUG: a fixture minor")
    }

    fn write(dir: &Path, path: &str, text: &str) -> PathBuf {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("BUG: a file in a directory"))
            .expect("BUG: a writable temporary directory");
        std::fs::write(&path, text).expect("BUG: a writable temporary directory");
        path
    }

    fn lockfile(dir: &Path, packages: &[(&str, &str)]) -> PathBuf {
        let packages: Vec<String> = packages
            .iter()
            .map(|(name, version)| {
                format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n")
            })
            .collect();
        write(dir, "Cargo.lock", &packages.join("\n"))
    }

    #[test]
    fn the_newest_stable_unyanked_release_of_the_minor_is_picked() {
        let lines = index(&[
            ("2.3.9", false),
            ("2.4.0", false),
            ("2.4.10", false),
            ("2.4.2", false),
            ("2.4.11", true),
            ("2.4.12-beta.1", false),
            ("2.5.0", false),
        ]);
        assert_eq!(
            newest_in_minor("icu_probe", &lines, &minor("2.4")).map(|v| v.to_string()),
            Ok("2.4.10".to_owned())
        );
    }

    #[test]
    fn a_minor_without_a_stable_release_is_refused_naming_the_crate() {
        let refused = newest_in_minor("icu_probe", &index(&[("2.6.0-rc.1", false)]), &minor("2.6"));
        assert_eq!(
            refused,
            Err("icu_probe has no stable release of 2.6".to_owned())
        );
    }

    #[test]
    fn a_minor_that_is_not_one_is_refused() {
        let refused = parse_version("two.0").expect_err("BUG: not a version");
        assert!(refused.contains("not a version"), "{refused}");
    }

    #[test]
    fn a_crate_is_found_where_the_sparse_index_files_it() {
        for (name, path) in [
            ("a", "1/a"),
            ("ab", "2/ab"),
            ("abc", "3/a/abc"),
            ("icu4x-datagen", "ic/u4/icu4x-datagen"),
            ("Serde", "se/rd/serde"),
        ] {
            assert_eq!(index_path(name), path);
        }
    }

    #[test]
    fn the_tested_cldr_tag_is_read_from_icu_provider_source() {
        for declaration in [
            "    pub const TESTED_CLDR_TAG: &'static str = \"49.0.0\";\n",
            "    pub const TESTED_CLDR_TAG: &str = \"49.0.0\";\n",
        ] {
            assert_eq!(cldr_tag_of(declaration), Ok("49.0.0".to_owned()));
        }
    }

    #[test]
    fn the_cldr_source_is_the_one_datagen_locks_not_the_newest() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let lock = lockfile(
            dir.path(),
            &[("icu_provider_source", "2.3.0"), ("icu_provider", "2.3.1")],
        );
        assert_eq!(
            locked_version(&lock, "icu_provider_source").map(|v| v.to_string()),
            Ok("2.3.0".to_owned())
        );
    }

    #[test]
    fn a_crate_locked_twice_has_no_one_version() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let lock = lockfile(
            dir.path(),
            &[
                ("icu_provider_source", "2.3.0"),
                ("icu_provider_source", "2.4.0"),
            ],
        );
        let refused = locked_version(&lock, "icu_provider_source").expect_err("BUG: locked twice");
        assert!(
            refused.contains("locks icu_provider_source 2 times"),
            "{refused}"
        );
    }

    #[test]
    fn a_release_tree_names_its_crates_versions() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let tree = dir.path();
        write(
            tree,
            "Cargo.toml",
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"1.9.0\"\n",
        );
        write(
            tree,
            "crates/parser/Cargo.toml",
            "[package]\nname = \"formatjs_icu_messageformat_parser\"\nversion = \"1.2.0\"\n",
        );
        write(
            tree,
            "crates/cli/Cargo.toml",
            "[package]\nname = \"formatjs_cli\"\nversion.workspace = true\n",
        );
        assert_eq!(
            workspace_versions(tree),
            Ok(BTreeMap::from([
                ("formatjs_cli".to_owned(), "1.9.0".to_owned()),
                (
                    "formatjs_icu_messageformat_parser".to_owned(),
                    "1.2.0".to_owned()
                ),
            ]))
        );
    }

    /// `[workspace]` split around another table, as the root `Cargo.toml` has it.
    const CARGO_TOML: &str = "\
[workspace]
members = [\"a\"]

[profile.release]
lto = true

[workspace.dependencies]
# Kept as written.
icu_provider = { version = \"2.3.1\", default-features = false, features = [\"sync\"] }
formatjs_intl_macros = \"1.1.1\"
other = \"1\"
";

    fn requirements(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, requirement)| ((*name).to_owned(), (*requirement).to_owned()))
            .collect()
    }

    #[test]
    fn requirements_are_set_and_nothing_else_moves() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let cargo_toml = write(dir.path(), "Cargo.toml", CARGO_TOML);
        set_requirements(
            &cargo_toml,
            &requirements(&[
                ("icu_provider", "~2.4.0"),
                ("formatjs_intl_macros", "1.2.0"),
            ]),
        )
        .expect("BUG: both are declared");
        assert_eq!(
            std::fs::read_to_string(&cargo_toml).expect("BUG: just written"),
            CARGO_TOML
                .replace("version = \"2.3.1\"", "version = \"~2.4.0\"")
                .replace(
                    "formatjs_intl_macros = \"1.1.1\"",
                    "formatjs_intl_macros = \"1.2.0\""
                )
        );
    }

    #[test]
    fn a_dotted_key_requirement_keeps_its_other_keys() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let dotted = "[workspace.dependencies]\nicu_provider.version = \"2.3.1\"\nicu_provider.features = [\"sync\"]\n";
        let cargo_toml = write(dir.path(), "Cargo.toml", dotted);
        set_requirements(&cargo_toml, &requirements(&[("icu_provider", "~2.4.0")]))
            .expect("BUG: declared");
        assert_eq!(
            std::fs::read_to_string(&cargo_toml).expect("BUG: just written"),
            dotted.replace("\"2.3.1\"", "\"~2.4.0\"")
        );
    }

    #[test]
    fn the_declared_icu4x_crates_are_read_from_the_manifest() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let cargo_toml = write(dir.path(), "Cargo.toml", CARGO_TOML);
        assert_eq!(
            declared_icu4x(&cargo_toml),
            Ok(vec!["icu_provider".to_owned()])
        );
    }

    #[test]
    fn an_undeclared_requirement_is_refused() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let cargo_toml = write(dir.path(), "Cargo.toml", CARGO_TOML);
        let refused = set_requirements(&cargo_toml, &requirements(&[("icu_unknown", "2.4.0")]));
        assert_eq!(
            refused,
            Err("Cargo.toml declares no icu_unknown".to_owned())
        );
    }

    #[test]
    fn every_locked_icu4x_crate_is_moved() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let lock = lockfile(
            dir.path(),
            &[
                ("icu_provider", "2.3.1"),
                ("icu", "2.3.1"),
                ("serde", "1.0.0"),
                ("icu_datetime", "2.3.1"),
            ],
        );
        assert_eq!(
            locked_icu4x(&lock),
            Ok(vec![
                "icu".to_owned(),
                "icu_datetime".to_owned(),
                "icu_provider".to_owned()
            ])
        );
    }

    #[test]
    fn a_vendor_build_naming_no_hash_leaves_the_pin_as_it_was() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let pin = write(dir.path(), "pin.nix", "  cargoHash = \"sha256-kept\";\n");
        let refused = probe_cargo_hash("probe", &pin, || {
            Ok("error: something else broke".to_owned())
        })
        .expect_err("BUG: no hash named");
        assert!(refused.contains("something else broke"), "{refused}");
        assert_eq!(
            nix_pin::get(&pin, "cargoHash"),
            Ok("sha256-kept".to_owned())
        );
    }

    #[test]
    fn the_vendor_hash_is_taken_over_another_fetchs() {
        let stderr = "error: hash mismatch in fixed-output derivation '/nix/store/x-source.drv':\n\
                      \x20        specified: sha256-A\n            got:    sha256-source\n\
                      error: hash mismatch in fixed-output derivation '/nix/store/y-probe-vendor-staging.drv':\n\
                      \x20        specified: sha256-A\n            got:    sha256-vendor\n";
        assert_eq!(vendor_hash(stderr), Some("sha256-vendor"));
    }

    #[test]
    fn an_unreadable_index_names_the_crate() {
        // The discard port: nothing listens there, so the connection is refused at once.
        let refused =
            read_index("icu_probe", "http://127.0.0.1:9/").expect_err("BUG: nothing listens");
        assert!(
            refused.contains("cannot read icu_probe from the crates.io index"),
            "{refused}"
        );
    }
}
