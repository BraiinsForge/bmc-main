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

//! Surfaces: source trees whose message sites are one extraction, each with its translator files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use clap::ValueEnum;
use serde::Deserialize;

use crate::console;

/// A crate that depends on one of these can declare host copy:
/// `format_message!` through the formatter, `message_descriptor!` through the lazy-message crate.
const MESSAGE_CRATES: [&str; 2] = ["bmc-intl", "bmc-intl-message"];

/// A surface as the command line names it, so a name
/// that is not one is refused before any work.
#[derive(Clone, Copy, ValueEnum)]
pub enum SurfaceName {
    /// Every crate that can declare copy.
    Host,
    /// The web app.
    Web,
}

#[derive(Debug)]
pub struct Surface {
    pub name: String,
    /// Globs of the sources, as `formatjs extract` takes them.
    pub sources: Vec<String>,
    /// The directory of `<key>.json` translator files.
    pub translations: PathBuf,
    /// Globs `formatjs extract` skips.
    pub ignore: Vec<String>,
}

impl Surface {
    /// One per language besides English; a language is added by adding its file.
    ///
    /// # Errors
    /// The directory named, when it is there but cannot be listed.
    pub fn translator_files(&self) -> Result<Vec<PathBuf>, String> {
        let listing = match std::fs::read_dir(&self.translations) {
            Ok(listing) => listing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(console::at(&self.translations, error)),
        };
        let mut files = Vec::new();
        for entry in listing {
            let path = entry
                .map_err(|error| console::at(&self.translations, error))?
                .path();
            if path.extension().is_some_and(|ext| ext == "json") {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }

    #[must_use]
    pub fn fix_command(&self) -> String {
        format!("just intl::extract {}", self.name)
    }
}

/// The language a translator file translates into, as its name says.
#[must_use]
pub fn language_of(translator_file: &Path) -> String {
    translator_file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The part of a `cargo metadata` package this reads.
#[derive(Deserialize)]
struct Package {
    name: String,
    manifest_path: PathBuf,
    dependencies: Vec<Dependency>,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

/// # Errors
/// Why git would not start.
pub fn git(dir: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| console::spawn_failed("git", &error))
}

/// The checkout the command runs in: its git toplevel, else the working directory.
/// Not derived from this binary, which the nix app runs from the store.
#[must_use]
pub fn repo_root() -> PathBuf {
    let current = std::env::current_dir().unwrap_or_default();
    git(&current, &["rev-parse", "--show-toplevel"])
        .ok()
        .filter(|output| output.status.success())
        .map_or(current, |output| {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        })
}

/// `path` as the checkout names it, when it is inside.
#[must_use]
pub fn relative<'a>(repo: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(repo).unwrap_or(path)
}

/// The named surfaces, or every surface when no names are given.
///
/// # Errors
/// Why `cargo metadata` failed, which the host surface is read from.
pub fn select(names: &[SurfaceName], repo: &Path) -> Result<Vec<Surface>, String> {
    let names = if names.is_empty() {
        SurfaceName::value_variants()
    } else {
        names
    };
    names.iter().map(|name| surface(*name, repo)).collect()
}

fn surface(name: SurfaceName, repo: &Path) -> Result<Surface, String> {
    Ok(match name {
        SurfaceName::Host => Surface {
            name: "host".to_owned(),
            sources: host_sources(&workspace_packages(repo)?),
            translations: repo.join("bmc-intl/core/lang"),
            ignore: Vec::new(),
        },
        SurfaceName::Web => Surface {
            name: "web".to_owned(),
            sources: vec![
                repo.join("frontend/src/**/*.{ts,tsx}")
                    .display()
                    .to_string(),
            ],
            translations: repo.join("frontend/lang"),
            // A spec's messages are fixtures, not copy.
            ignore: vec!["**/*.d.ts".to_owned(), "**/*.spec.{ts,tsx}".to_owned()],
        },
    })
}

/// Every member of the root workspace, as `cargo metadata` describes it.
///
/// # Errors
/// What `cargo metadata` printed when it failed.
fn workspace_packages(repo: &Path) -> Result<Vec<Package>, String> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
        ])
        .arg(repo.join("Cargo.toml"))
        .output()
        .map_err(|error| console::spawn_failed("cargo", &error))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned());
    }
    serde_json::from_slice::<Metadata>(&output.stdout)
        .map(|metadata| metadata.packages)
        .map_err(|error| format!("cargo metadata: {error}"))
}

/// The `src/**/*.rs` glob of every package that can declare host copy:
/// one with a [`MESSAGE_CRATES`] member among its normal dependencies.
///
/// A proc macro runs at build time and shows nothing on a device,
/// so it is passed over, as is a crate whose dependency is only
/// for its tests, which are fixtures, not copy.
///
/// The message crates themselves are left out,
/// their own tests' messages being fixtures too.
#[must_use]
fn host_sources(packages: &[Package]) -> Vec<String> {
    let mut sources: Vec<String> = packages
        .iter()
        .filter(|package| !MESSAGE_CRATES.contains(&package.name.as_str()))
        .filter(|package| {
            !package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|kind| kind == "proc-macro"))
        })
        .filter(|package| {
            package
                .dependencies
                .iter()
                .any(|dep| MESSAGE_CRATES.contains(&dep.name.as_str()) && dep.kind.is_none())
        })
        .map(|package| {
            package
                .manifest_path
                .parent()
                .unwrap_or(Path::new(""))
                .join("src/**/*.rs")
                .display()
                .to_string()
        })
        .collect();
    sources.sort();
    sources
}

/// A surface named `probe` over `root`'s `sources` glob, its translator files in `root/lang`.
#[cfg(test)]
pub fn probe_surface(root: &Path, sources: &str) -> Surface {
    Surface {
        name: "probe".to_owned(),
        sources: vec![root.join(sources).display().to_string()],
        translations: root.join("lang"),
        ignore: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(
        root: &Path,
        name: &str,
        path: &str,
        deps: &[&str],
        kind: &str,
        dep_kind: Option<&str>,
    ) -> Package {
        Package {
            name: name.to_owned(),
            manifest_path: root.join(path).join("Cargo.toml"),
            dependencies: deps
                .iter()
                .map(|dep| Dependency {
                    name: (*dep).to_owned(),
                    kind: dep_kind.map(str::to_owned),
                })
                .collect(),
            targets: vec![Target {
                kind: vec![kind.to_owned()],
            }],
        }
    }

    #[test]
    fn the_web_surface_is_its_fix_commands_name() {
        let surfaces =
            select(&[SurfaceName::Web], Path::new("/repo")).expect("BUG: the web reads no cargo");
        let [web] = &surfaces[..] else {
            panic!("only the surface named: {surfaces:?}");
        };
        assert_eq!(web.fix_command(), "just intl::extract web");
    }

    #[test]
    fn a_translator_directory_that_cannot_be_listed_is_named() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let lang = dir.path().join("lang");
        std::fs::write(&lang, "").expect("BUG: a writable temporary directory");
        let surface = Surface {
            translations: lang,
            ..probe_surface(dir.path(), "src/**/*.rs")
        };
        let refused = surface.translator_files();
        assert!(
            refused.as_ref().is_err_and(|error| error.contains("lang")),
            "{refused:?}"
        );
    }

    #[test]
    fn the_host_sources_are_the_crates_that_can_declare_copy() {
        let root = Path::new("/repo");
        let packages = [
            package(
                root,
                "bmc-field-schema",
                "bmc-field-schema",
                &["anyhow", "bmc-intl-message"],
                "lib",
                None,
            ),
            package(root, "bmc", "bmc", &["bmc-intl"], "lib", None),
            package(
                root,
                "bmc-overlay-probe",
                "system-overlays/bmc-overlay-probe",
                &["bmc-intl"],
                "lib",
                None,
            ),
            package(
                root,
                "bmc-render-macros",
                "bmc-render/macros",
                &["bmc-intl"],
                "proc-macro",
                None,
            ),
            package(
                root,
                "bmc-intl",
                "bmc-intl/core",
                &["bmc-intl-message"],
                "lib",
                None,
            ),
            package(root, "bmc-audio", "bmc-audio", &["anyhow"], "lib", None),
            package(
                root,
                "bmc-scheduler",
                "bmc-scheduler",
                &["bmc-intl"],
                "lib",
                Some("dev"),
            ),
        ];
        assert_eq!(
            host_sources(&packages),
            [
                "/repo/bmc-field-schema/src/**/*.rs",
                "/repo/bmc/src/**/*.rs",
                "/repo/system-overlays/bmc-overlay-probe/src/**/*.rs",
            ]
        );
    }
}
