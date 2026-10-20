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

//! Compiles every `lang/<key>.json` translator file into `CATALOGS: &[(&str, &str)]`,
//! the `(catalog key, {id: translation} JSON)` pairs `Translations::bundled` loads,
//! so a language is added by adding its file. An absent directory is no languages.

use std::path::{Path, PathBuf};
use std::{env, fs};

include!("build/compile.rs");

fn main() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lang = crate_dir.join("lang");
    // A missing path counts as changed on every build; the crate directory notices `lang/` appear.
    let watched = if lang.is_dir() { &lang } else { crate_dir };
    println!("cargo:rerun-if-changed={}", watched.display());

    let entries: Vec<String> = translator_files(&lang)
        .iter()
        .map(|path| {
            let key = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("BUG: a `.json` path has a UTF-8 stem");
            let text = fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", path.display()));
            let catalog = compile(&path.display().to_string(), &text)
                .unwrap_or_else(|error| panic!("{error}"));
            let json = serde_json::to_string(&catalog).expect("BUG: a string map serialises");
            format!("({key:?}, {json:?}),")
        })
        .collect();
    let generated = format!(
        "pub(crate) const CATALOGS: &[(&str, &str)] = &[\n{}\n];\n",
        entries.join("\n")
    );
    let out = Path::new(&env::var("OUT_DIR").expect("BUG: cargo sets OUT_DIR")).join("catalogs.rs");
    fs::write(out, generated).expect("BUG: OUT_DIR is writable");
}

fn translator_files(lang: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(lang)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}
