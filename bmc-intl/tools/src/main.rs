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

//! Translator files of every surface:
//!  - intl extract
//!  - intl verify
//!  - intl untranslated
//!
//! A *surface* is a source tree whose message sites are one extraction,
//! with a translator file per language: `bmc-intl/core/lang/<key>.json`
//! for `host`, every crate that can declare copy, and `frontend/lang/<key>.json`
//! for `web`, the web app. Each build compiles the files into its own catalog.
//!
//! `formatjs` is the CLI `nix/intl/formatjs-cli.nix` builds, on PATH in the dev shell.

mod check;
mod console;
mod extract;
mod formatjs;
mod surface;
mod translator;
mod untranslated;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::builder::styling::{AnsiColor, Styles};
use clap::{Args, Parser, Subcommand};

use crate::surface::SurfaceName;

/// Cargo's help, with errors in the red `just` reports in,
/// so a failed run reads in one colour.
const HELP: Styles = clap_cargo::style::CLAP_STYLING.error(AnsiColor::Red.on_default().bold());

#[derive(Parser)]
#[command(
    name = "intl",
    about = "Translator files of every surface: extract, verify, and list what waits for a translator",
    styles = HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Checkout to work in; its git toplevel when omitted.
    #[arg(long, global = true)]
    repo: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Extract messages and sync every translator file by id.
    ///
    /// A translation is kept by id, else by identical English
    /// when exactly one gone translated entry and one new message share it;
    /// new messages get `"translation": null`, removed ones go;
    /// `source` and `description` are refreshed.
    ///
    /// Never writes a translation.
    /// A language is added by adding an empty `lang/<key>.json`.
    Extract(Surfaces),
    /// Fail on a surface whose translator files are stale or broken.
    ///
    /// The translator files must hold exactly the extracted messages
    /// with current `source` and `description`, and every translation
    /// must parse as the runtime parses it, with the English's arguments
    /// of the same types.
    ///
    /// An untranslated message fails nothing;
    /// the surface's line counts them.
    /// Exit 1 on any finding.
    Verify(Surfaces),
    /// List the messages still waiting for a translation, per surface and language.
    ///
    /// An untranslated message shows in English and fails nothing.
    /// With `--since`, only those added or reworded after that commit
    /// are listed, and any of them is exit 1: the merge request's warning.
    Untranslated {
        #[command(flatten)]
        surfaces: Surfaces,
        /// A commit to list only what changed after,
        /// or `merge-base` for the merge request's base.
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Args)]
struct Surfaces {
    /// Surfaces to work on; every surface when omitted.
    surfaces: Vec<SurfaceName>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let repo = cli.repo.unwrap_or_else(surface::repo_root);
    console::blank();
    let ran = match &cli.command {
        Command::Extract(Surfaces { surfaces }) => extract::run(surfaces, &repo),
        Command::Verify(Surfaces { surfaces }) => verify::run(surfaces, &repo),
        Command::Untranslated {
            surfaces: Surfaces { surfaces },
            since,
        } => untranslated::run(surfaces, since.as_deref(), &repo),
    };
    let code = ran.unwrap_or_else(|error| {
        console::error(&error);
        ExitCode::FAILURE
    });
    console::blank();
    code
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::CommandFactory;

    use super::*;
    use crate::surface::{Surface, probe_surface};
    use crate::translator::{read_entries, write_json};

    /// The id pattern the checkout hashes by, read as the tool reads it.
    fn id_pattern() -> String {
        formatjs::id_pattern(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .expect("BUG: the checkout's message-id.json")
    }

    /// What `formatjs` extracts from `surface`; it is on PATH in the dev shell and CI.
    fn extract(surface: &Surface) -> translator::Extraction {
        formatjs::extract(surface, &id_pattern()).expect("BUG: an extraction")
    }

    /// The English of every message `formatjs` extracts from `surface`.
    fn messages(surface: &Surface) -> Vec<String> {
        extract(surface)
            .into_values()
            .map(|extracted| extracted.message)
            .collect()
    }

    #[test]
    fn the_cli_exposes_every_subcommand() {
        let names: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect();
        assert_eq!(names, ["extract", "verify", "untranslated"]);
    }

    #[test]
    fn a_name_that_is_no_surface_is_refused_with_those_there_are() {
        let refused = Cli::try_parse_from(["intl", "verify", "nope"])
            .err()
            .expect("BUG: `nope` names no surface");
        assert_eq!(refused.kind(), clap::error::ErrorKind::InvalidValue);
        let text = refused.to_string();
        assert!(
            text.contains("'nope'") && text.contains("host") && text.contains("web"),
            "{text}"
        );
    }

    #[test]
    fn the_macro_hashes_a_message_to_the_id_every_build_uses() {
        let message = bmc_intl_message::message_descriptor!(
            default_message: "Probe",
            description: "Fixture for the message id hash"
        );
        assert_eq!(message.id, "64/ZPeJ+Sp");
    }

    /// `root`'s `src/lib.rs` holding `file`, written as rustfmt-style source.
    fn write_rust(root: &Path, file: &proc_macro2::TokenStream) {
        let source = prettyplease::unparse(
            &syn::parse2(file.clone()).expect("BUG: the fixture is a Rust file"),
        );
        write_source(root, "src/lib.rs", &source);
    }

    /// `text` written to `path` under `root`, its directory made as needed.
    fn write_source(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("BUG: a source file has a directory"))
            .expect("BUG: a writable temporary directory");
        std::fs::write(path, text).expect("BUG: a writable temporary directory");
    }

    fn rust_surface(root: &Path, english: &str) -> Surface {
        write_rust(
            root,
            &quote::quote! {
                fn f(intl: I) {
                    format_message!(intl, default_message: #english, description: "Greeting", values: { name: n });
                }
            },
        );
        std::fs::create_dir_all(root.join("lang")).expect("BUG: a writable temporary directory");
        probe_surface(root, "src/**/*.rs")
    }

    #[test]
    fn the_cli_extracts_the_id_the_macro_bakes_in() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        write_rust(
            root,
            &quote::quote! {
                fn f(intl: I) {
                    format_message!(intl, default_message: "Probe", description: "Fixture for the message id hash");
                }
            },
        );
        let extracted = extract(&probe_surface(root, "src/**/*.rs"));
        assert_eq!(extracted.keys().collect::<Vec<_>>(), ["64/ZPeJ+Sp"]);
    }

    #[test]
    fn round_trip_through_formatjs() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        let surface = rust_surface(root, "Hello {name}");
        let translator_file = root.join("lang/zh.json");
        std::fs::write(&translator_file, "{}\n").expect("BUG: a writable temporary directory");
        let pattern = id_pattern();

        extract::extract_surface(&surface, &pattern).expect("BUG: an extraction");
        let mut entries = read_entries(&translator_file).expect("BUG: just written");
        let id = entries.keys().next().cloned().expect("BUG: one message");
        assert_eq!(entries[&id].source, "Hello {name}");
        assert_eq!(entries[&id].description.as_deref(), Some("Greeting"));
        assert_eq!(
            findings(&surface, &pattern),
            Vec::<String>::new(),
            "an untranslated message fails nothing"
        );

        entries.get_mut(&id).expect("BUG: present").translation = Some("你好 {name}".to_owned());
        write_json(&translator_file, &entries).expect("BUG: writable");
        assert_eq!(findings(&surface, &pattern), Vec::<String>::new());

        entries.get_mut(&id).expect("BUG: present").translation = Some("你好 {nom}".to_owned());
        write_json(&translator_file, &entries).expect("BUG: writable");
        let checked = verify::verify_surface(&surface, &pattern).expect("BUG: readable");
        let [file] = &checked[..] else {
            panic!("one translator file: {checked:?}");
        };
        assert_eq!(file.file, translator_file);
        assert!(
            matches!(&file.problems[..], [one] if one.contains(&id)),
            "{checked:?}"
        );

        let _ = rust_surface(root, "Hello {name}!");
        let problems = findings(&surface, &pattern);
        assert!(
            problems.iter().any(|p| p.contains("missing entry")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains(&format!("entry {id} no longer in the source"))),
            "{problems:?}"
        );
    }

    /// Every problem `verify` finds on `surface`, whichever translator file it is in.
    fn findings(surface: &Surface, pattern: &str) -> Vec<String> {
        verify::verify_surface(surface, pattern)
            .expect("BUG: readable")
            .into_iter()
            .flat_map(|file| file.problems)
            .collect()
    }

    #[test]
    fn extract_twice_leaves_the_translator_file_byte_identical() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        let surface = rust_surface(root, "Hello {name}");
        let translator_file = root.join("lang/zh.json");
        std::fs::write(&translator_file, "{}\n").expect("BUG: a writable temporary directory");
        let pattern = id_pattern();
        extract::extract_surface(&surface, &pattern).expect("BUG: an extraction");
        let first = std::fs::read(&translator_file).expect("BUG: just written");
        extract::extract_surface(&surface, &pattern).expect("BUG: an extraction");
        assert_eq!(
            std::fs::read(&translator_file).expect("BUG: just written"),
            first
        );
    }

    #[test]
    fn a_web_surface_extracts_typescript_and_skips_ignored_files() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        write_source(
            root,
            "src/app.tsx",
            "const t = intl.formatMessage({ defaultMessage: 'Hello {name}' });\n",
        );
        write_source(
            root,
            "src/types.d.ts",
            "declare const x: string; intl.formatMessage({ defaultMessage: 'Not me' });\n",
        );
        let surface = Surface {
            ignore: vec!["**/*.d.ts".to_owned()],
            ..probe_surface(root, "src/**/*.{ts,tsx}")
        };
        assert_eq!(messages(&surface), ["Hello {name}"]);
    }

    #[test]
    fn a_format_message_handed_in_as_a_parameter_is_extracted() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        write_source(
            root,
            "src/steps.ts",
            "const label = (formatMessage: F) => formatMessage({ defaultMessage: 'Downloading' });\n",
        );
        assert_eq!(
            messages(&probe_surface(root, "src/**/*.ts")),
            ["Downloading"]
        );
    }

    #[test]
    fn verify_counts_what_waits_on_the_surface_line() {
        let checked = |language: &str, untranslated| verify::Checked {
            file: format!("lang/{language}.json").into(),
            untranslated,
            problems: Vec::new(),
        };
        let line = verify::summary("probe", &[checked("de", 0), checked("zh", 2)]);
        assert_eq!(
            ::console::strip_ansi_codes(&line),
            "probe: translator files current; untranslated: 2 in zh"
        );
    }
}
