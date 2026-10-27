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

//! Language packs read from directories at run time.
//!
//! A root holds `<lang>/` directories, each with:
//! the translator files `system.json`, `web.json` and `widgets/<uid>.json`;
//! the ICU4X blob `cldr.postcard` and the unit patterns `units.postcard`;
//! the language's own name in `info.json`; and its flag as `flag.svg` and compiled as `flag.bin`.
//!
//! A later root overrides an earlier one per entry.
//! A regional pack's translations layer over its parent's the same way:
//! `zh-SG` brings only what differs from `zh`, and its entry wins over its parent's from any root.
//! Its CLDR data, unit patterns, name and flags are its own.
//!
//! A pack's facts — its name, `info.json` and flags — are read up front, the rest on first use.
//! A file or entry that does not parse is left out and reported, never fatal.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use bmc_shared_utils::asset::is_lfs_pointer;
use bmc_ui::svg::{compiled_svg_len, svg_viewbox};
use formatjs_intl::Messages;
use icu_locale::LocaleExpander;
use icu_locale_core::LanguageIdentifier;
use icu_locale_core::subtags::{Script, script};
use icu_provider_blob::BlobDataProvider;

use crate::{Data, Language, units};

/// Colon-separated roots, read in order; unset, [`DEFAULT_ROOTS`].
pub const ROOTS_VAR: &str = "BMC_LANGUAGES_PATH";

/// On the data partition, so dropped-in packs never press on the overlay.
pub const LOCAL_ROOT: &str = "/mnt/data/bmc/languages";

pub const DEFAULT_ROOTS: [&str; 2] = ["/run/current-profile/languages", LOCAL_ROOT];

pub const MAX_FLAG_BYTES: u64 = 65_536;

/// A pack's ICU4X blob.
const CLDR_FILE: &str = "cldr.postcard";

#[must_use]
pub fn roots() -> Vec<PathBuf> {
    std::env::var_os(ROOTS_VAR).map_or_else(
        || DEFAULT_ROOTS.iter().map(PathBuf::from).collect(),
        |roots| std::env::split_paths(&roots).collect(),
    )
}

/// What a load accepted, refused and skipped.
#[derive(Clone, Debug, Default)]
pub struct LoadReport {
    /// Beyond English.
    pub languages: Vec<Language>,
    pub refused: Vec<(Language, String)>,
    pub skipped: Vec<String>,
    /// Translations the runtime would not parse.
    pub invalid: Vec<String>,
}

impl LoadReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.refused.is_empty() && self.skipped.is_empty() && self.invalid.is_empty()
    }

    fn skip(&mut self, path: &Path, why: impl fmt::Display) {
        self.skipped.push(format!("{}: {why}", path.display()));
    }

    /// A parent pack's findings come again with every pack layered over it; each is kept once.
    pub(crate) fn extend(&mut self, other: &Self) {
        fn once<T: Clone + PartialEq>(into: &mut Vec<T>, from: &[T]) {
            for item in from {
                if !into.contains(item) {
                    into.push(item.clone());
                }
            }
        }
        once(&mut self.languages, &other.languages);
        once(&mut self.refused, &other.refused);
        once(&mut self.skipped, &other.skipped);
        once(&mut self.invalid, &other.invalid);
    }
}

impl fmt::Display for LoadReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "languages: {}", Language::EN)?;
        for language in &self.languages {
            write!(f, ", {language}")?;
        }
        for (language, reason) in &self.refused {
            write!(f, "\nrefused {language}: {reason}")?;
        }
        for line in &self.skipped {
            write!(f, "\nskipped {line}")?;
        }
        for line in &self.invalid {
            write!(f, "\ninvalid {line}")?;
        }
        Ok(())
    }
}

/// A pack's `info.json`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct LanguageInfo {
    /// `Čeština`, `简体中文`.
    #[serde(rename = "name")]
    pub native_name: String,
    /// `ltr` when omitted, even over an earlier root's: a later `info.json` replaces it whole.
    #[serde(rename = "direction", default)]
    pub text_direction: TextDirection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    #[default]
    Ltr,
    Rtl,
}

/// The scripts the shipped fonts draw; Han only as Simplified Chinese writes it.
const DRAWN_SCRIPTS: [Script; 4] = [
    script!("Latn"),
    script!("Grek"),
    script!("Cyrl"),
    script!("Hans"),
];

/// Text in a script the fonts lack draws as boxes.
fn script_refusal(language: Language) -> Option<String> {
    static EXPANDER: LazyLock<LocaleExpander> = LazyLock::new(|| Data::builtin().expander());
    let mut identifier = LanguageIdentifier::from(language);
    EXPANDER.maximize(&mut identifier);
    match identifier.script {
        Some(script) if DRAWN_SCRIPTS.contains(&script) => None,
        Some(script) => Some(format!(
            "written in {script}, which the device's fonts do not draw"
        )),
        None => Some("its script is unknown; name the pack with one, such as `xx-Latn`".to_owned()),
    }
}

/// What packs say of themselves without opening their translations or CLDR data.
#[derive(Default)]
pub(crate) struct Facts {
    /// In root order.
    pub(crate) dirs: BTreeMap<Language, Vec<PathBuf>>,
    /// As `dirs` before any refusal: a refused pack still lends its text to those layered over it.
    pub(crate) text_dirs: BTreeMap<Language, Vec<PathBuf>>,
    pub(crate) presentation: Presentation,
}

#[derive(Default)]
pub(crate) struct Presentation {
    pub(crate) info: BTreeMap<Language, LanguageInfo>,
    pub(crate) flags_svg: BTreeMap<Language, PathBuf>,
    pub(crate) flags_bin: BTreeMap<Language, PathBuf>,
}

impl Presentation {
    pub(crate) fn remove(&mut self, language: Language) {
        self.info.remove(&language);
        self.flags_svg.remove(&language);
        self.flags_bin.remove(&language);
    }
}

/// What a pack brings beyond its facts.
#[derive(Default)]
pub(crate) struct Contents {
    pub(crate) system: Messages,
    pub(crate) web: Option<Messages>,
    pub(crate) widgets: BTreeMap<String, Messages>,
    /// The `web.json` and `widgets/<uid>.json` its own directories bring, not its layers.
    pub(crate) own_files: BTreeSet<String>,
    pub(crate) blob: Option<BlobDataProvider>,
    pub(crate) units: Option<units::Loaded>,
}

pub(crate) fn read(roots: &[PathBuf], report: &mut LoadReport) -> Facts {
    let mut found = Facts::default();
    for root in roots {
        for dir in sorted_entries(root, report)
            .into_iter()
            .filter(|path| path.is_dir())
        {
            read_language(&dir, &mut found, report);
        }
    }
    found.text_dirs = found.dirs.clone();
    // Judged on the `info.json` in effect after every root.
    let right_to_left: Vec<Language> = found
        .presentation
        .info
        .iter()
        .filter(|(_, info)| info.text_direction == TextDirection::Rtl)
        .map(|(language, _)| *language)
        .collect();
    for language in right_to_left {
        found.dirs.remove(&language);
        found.presentation.remove(language);
        report.refused.push((
            language,
            "written right to left, which the device does not lay out yet".to_owned(),
        ));
    }
    // A pack brings its own CLDR data: none falls back to another's, the base blob's included.
    let without_cldr: Vec<Language> = found
        .dirs
        .iter()
        .filter(|(_, dirs)| !dirs.iter().any(|dir| dir.join(CLDR_FILE).is_file()))
        .map(|(language, _)| *language)
        .collect();
    for language in without_cldr {
        found.dirs.remove(&language);
        found.presentation.remove(language);
        report.refused.push((
            language,
            format!("no `{CLDR_FILE}` in any of its directories; a pack brings its own CLDR data"),
        ));
    }
    found
}

fn read_language(dir: &Path, found: &mut Facts, report: &mut LoadReport) {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // One name per language: a second spelling would file a second catalog nothing reads.
    let key = match Language::parse(&name).map(crate::pack_language) {
        Ok(key) if key.tag() == name => key,
        Ok(key) => {
            report.skip(dir, format_args!("name it `{key}`"));
            return;
        }
        Err(error) => {
            report.skip(dir, format_args!("not a BCP-47 tag: {error}"));
            return;
        }
    };
    if key == Language::EN {
        report.skip(dir, "English is the code's own");
        return;
    }
    // `und` ends every fallback chain: its catalog would answer any language without a pack.
    if LanguageIdentifier::from(key).language.is_unknown() {
        report.skip(dir, "names no language");
        return;
    }
    if let Some(reason) = script_refusal(key) {
        if !report.refused.iter().any(|(refused, _)| *refused == key) {
            report.refused.push((key, reason));
        }
        return;
    }
    found.dirs.entry(key).or_default().push(dir.to_path_buf());
    let shown = &mut found.presentation;
    if let Some(flag) = flag_file(dir.join("flag.svg"), report) {
        shown.flags_svg.insert(key, flag);
    }
    if let Some(flag) =
        flag_file(dir.join("flag.bin"), report).filter(|flag| is_compiled_flag(flag, report))
    {
        shown.flags_bin.insert(key, flag);
    }
    if let Some(info) = read_info(&dir.join("info.json"), report) {
        shown.info.insert(key, info);
    }
}

/// `layers` lie beneath `dirs` for translations only.
pub(crate) fn read_contents(
    layers: &[PathBuf],
    dirs: &[PathBuf],
    report: &mut LoadReport,
) -> Contents {
    let mut contents = Contents::default();
    for dir in layers.iter().chain(dirs) {
        let own = dirs.contains(dir);
        if let Some(messages) = read_translations(&dir.join("system.json"), report) {
            contents.system.extend(messages);
        }
        if let Some(messages) = read_translations(&dir.join("web.json"), report) {
            contents.web.get_or_insert_default().extend(messages);
            if own {
                contents.own_files.insert("web.json".to_owned());
            }
        }
        for file in sorted_entries(&dir.join("widgets"), report)
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        {
            let uid = file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(messages) = read_translations(&file, report) {
                if own {
                    contents.own_files.insert(format!("widgets/{uid}.json"));
                }
                contents.widgets.entry(uid).or_default().extend(messages);
            }
        }
    }
    for dir in dirs {
        if let Some(blob) = read_blob(&dir.join(CLDR_FILE), report) {
            contents.blob = Some(blob);
        }
        if let Some(units) = read_units(&dir.join(units::FILE), report) {
            contents.units = Some(units);
        }
    }
    contents
}

/// Nothing when there is no such directory; a listing that fails otherwise is reported.
fn sorted_entries(dir: &Path, report: &mut LoadReport) -> Vec<PathBuf> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            report.skip(dir, format_args!("unreadable: {error}"));
            return Vec::new();
        }
    };
    let mut paths = Vec::new();
    for entry in listing {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => report.skip(dir, format_args!("unreadable: {error}")),
        }
    }
    paths.sort();
    paths
}

/// `None` when there is no such file, or it cannot be read, which is reported.
fn read_bytes(path: &Path, report: &mut LoadReport) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            report.skip(path, format_args!("unreadable: {error}"));
            None
        }
    }
}

fn read_text(path: &Path, report: &mut LoadReport) -> Option<String> {
    String::from_utf8(read_bytes(path, report)?)
        .map_err(|error| report.skip(path, format_args!("unreadable: {error}")))
        .ok()
}

fn read_info(path: &Path, report: &mut LoadReport) -> Option<LanguageInfo> {
    serde_json::from_str(&read_text(path, report)?)
        .map_err(|error| report.skip(path, format_args!("not a pack's info: {error}")))
        .ok()
}

fn flag_file(path: PathBuf, report: &mut LoadReport) -> Option<PathBuf> {
    let meta = std::fs::metadata(&path)
        .ok()
        .filter(std::fs::Metadata::is_file)?;
    if meta.len() > MAX_FLAG_BYTES {
        report.skip(
            &path,
            format_args!(
                "{} bytes, past the {MAX_FLAG_BYTES} a flag may have",
                meta.len()
            ),
        );
        return None;
    }
    Some(path)
}

const LFS_POINTER: &str = "a git-lfs pointer, not the file; run `git lfs pull`";

/// Whole and with a drawable viewBox; what its paths draw is the renderer's to judge.
fn is_compiled_flag(path: &Path, report: &mut LoadReport) -> bool {
    read_parsed(path, report, "a compiled flag", |bytes| {
        if svg_viewbox(&bytes).is_none() {
            Err("no drawable viewBox")
        } else if compiled_svg_len(&bytes) == Some(bytes.len()) {
            Ok(())
        } else {
            Err("cut short, run on past its end, or an op the renderer does not know")
        }
    })
    .is_some()
}

/// `None` when there is no such file or it does not parse, so an earlier root's stands.
fn read_parsed<T, E: fmt::Display>(
    path: &Path,
    report: &mut LoadReport,
    what: &str,
    parse: impl FnOnce(Vec<u8>) -> Result<T, E>,
) -> Option<T> {
    let bytes = read_bytes(path, report)?;
    let problem = if is_lfs_pointer(&bytes) {
        LFS_POINTER.to_owned()
    } else {
        match parse(bytes) {
            Ok(parsed) => return Some(parsed),
            Err(error) => error.to_string(),
        }
    };
    report.skip(path, format_args!("not {what}: {problem}"));
    None
}

fn read_blob(path: &Path, report: &mut LoadReport) -> Option<BlobDataProvider> {
    read_parsed(path, report, "an ICU4X blob", |bytes| {
        BlobDataProvider::try_new_from_blob(bytes.into_boxed_slice())
    })
}

fn read_units(path: &Path, report: &mut LoadReport) -> Option<units::Loaded> {
    read_parsed(path, report, "a language's units", |bytes| {
        units::load(bytes.into())
    })
}

/// `id → translation`; `None` when there is no such file or it is not an object.
fn read_translations(path: &Path, report: &mut LoadReport) -> Option<Messages> {
    let text = read_text(path, report)?;
    let entries: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text)
        .map_err(|error| report.skip(path, format_args!("not a JSON object: {error}")))
        .ok()?;
    Some(
        entries
            .into_iter()
            .filter_map(|(id, entry)| {
                match entry.as_object().map(|entry| entry.get("translation")) {
                    None => {
                        report.skip(path, format_args!("`{id}` is not an entry object"));
                        None
                    }
                    Some(None) => {
                        report.skip(path, format_args!("`{id}` has no `translation`"));
                        None
                    }
                    Some(Some(serde_json::Value::String(translation))) => {
                        let source = entry.get("source").and_then(serde_json::Value::as_str);
                        if let Err(error) = crate::check_translation(source, translation) {
                            report
                                .invalid
                                .push(format!("{}: `{id}`: {error}", path.display()));
                            return None;
                        }
                        Some((id, translation.clone()))
                    }
                    Some(Some(serde_json::Value::Null)) => None,
                    Some(Some(other)) => {
                        report.skip(path, format_args!("`{id}` is translated as {other}"));
                        None
                    }
                }
            })
            .collect(),
    )
}
