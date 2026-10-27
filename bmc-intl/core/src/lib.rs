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

//! Locale selection and the translations language packs bring.
//!
//! English is the source: every message carries its `default_message` inline and `en` has no catalog.
//! Every other language is a pack read at run time ([`packs`]), with the CLDR data to format it by.
//!
//! A language's pack is filed under [`pack_language`], where negotiation starts its fallback chain:
//! `zh` for Simplified Chinese, as `zh-Hans`, `zh-CN` and `zh-Hans-CN` all fall back to it.
//! A catalog keyed `zh-Hans` is never a fallback candidate and can never be selected.
//!
//! Copy is declared where it is used, its placeholders checked against the English at compile time:
//!
//! ```
//! let intl = bmc_intl::formatter(bmc_intl::Language::EN);
//! let text = bmc_intl::format_message!(intl, default_message: "Connect to {ssid}", values: { ssid: "deck" });
//! assert_eq!(text, "Connect to deck");
//! ```
//!
//! A placeholder the English has and the call does not give fails the build:
//!
//! ```compile_fail
//! let intl = bmc_intl::formatter(bmc_intl::Language::EN);
//! bmc_intl::format_message!(intl, default_message: "Connect to {ssid}");
//! ```
//!
//! So does a value the English has no placeholder for:
//!
//! ```compile_fail
//! let intl = bmc_intl::formatter(bmc_intl::Language::EN);
//! bmc_intl::format_message!(intl, default_message: "Connect", values: { ssid: "deck" });
//! ```

mod data;
mod formatter;
mod localized;
pub mod packs;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError, RwLock};

pub use bmc_intl_message::Message;
pub use bmc_intl_units as units;
pub use bmc_shared_utils::language::Language;
pub use data::Data;
use formatjs_icu_messageformat::Options;
pub use formatjs_icu_messageformat::{Value, Values};
use formatjs_icu_messageformat_parser::error::ParserError;
use formatjs_icu_messageformat_parser::parser::Parser;
use formatjs_icu_messageformat_parser::{MessageFormatElement, message_arguments};
use formatjs_intl::{IntlContext, MessageCatalog, MessageDescriptorRef};
pub use formatjs_intl::{Messages, format_message};
pub use formatter::Intl;
use icu_locale::LocaleFallbacker;
use icu_locale::fallback::LocaleFallbackConfig;
use icu_locale_core::LanguageIdentifier;
pub use localized::Localized;
pub use packs::{LanguageInfo, LoadReport, TextDirection};

const DEFAULT_LOCALE: &str = "en";

/// `pattern` parsed exactly as the runtime parses it.
fn parse_pattern(pattern: &str) -> Result<Vec<MessageFormatElement>, ParserError> {
    Parser::new(pattern, Options::default_parser_options()).parse()
}

/// Whether `translation` names exactly the arguments its `source` names, across every branch:
/// one it adds goes unfilled when the message formats, and one it drops loses a value.
///
/// A source that is not a pattern cannot be compared and passes.
fn same_arguments(source: &str, translation: &[MessageFormatElement]) -> Result<(), String> {
    let Ok(source) = parse_pattern(source) else {
        return Ok(());
    };
    let names = |ast: &[MessageFormatElement]| -> BTreeSet<String> {
        message_arguments(ast).into_keys().collect()
    };
    let (wanted, given) = (names(&source), names(translation));
    if wanted == given {
        return Ok(());
    }
    let braced = |names: &BTreeSet<String>| {
        names
            .iter()
            .map(|name| format!("{{{name}}}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(format!(
        "the source's arguments are [{}], the translation's [{}]",
        braced(&wanted),
        braced(&given)
    ))
}

/// A pack's translation as the runtime will take it: ICU MessageFormat that parses,
/// with the same arguments as the `source` beside it when the entry carries one.
pub(crate) fn check_translation(source: Option<&str>, translation: &str) -> Result<(), String> {
    let parsed = parse_pattern(translation).map_err(|error| error.to_string())?;
    source.map_or(Ok(()), |source| same_arguments(source, &parsed))
}

/// Word a [`Message`] declared elsewhere in `intl`'s language,
/// its translation by id or its English default, as `format_message!` does.
#[must_use]
pub fn format_lazy(intl: &Intl, message: &Message) -> String {
    let mut descriptor = MessageDescriptorRef::new(message.id, message.default_message);
    if let Some(description) = message.description {
        descriptor = descriptor.with_description(description);
    }
    intl.format_message_to_string_or_default_ref(descriptor, &Values::new())
}

/// A formatter over the translations in effect for `language`.
///
/// Not free: building one negotiates the language against the catalogs,
/// walking an ICU4X fallback chain and allocating a string per candidate.
/// Hold the result and build a new one when the language changes — never per render.
#[must_use]
pub fn formatter(language: Language) -> Intl {
    Translations::shared().formatter(language)
}

/// The process-wide formatter for `language`, built on first use and again after a reload:
/// for code answering many callers in many languages at once (a request handler)
/// which has no one language to hold a [`Localized`] for.
#[must_use]
pub fn shared_formatter(language: Language) -> Arc<Intl> {
    /// The formatters built over one [`Translations::version`].
    type Formatters = (u64, HashMap<Language, Arc<Intl>>);
    static EVERY: Mutex<Option<Formatters>> = Mutex::new(None);
    let current = Translations::shared();
    let mut every = EVERY.lock().unwrap_or_else(PoisonError::into_inner);
    let (version, formatters) = every.get_or_insert_with(|| (current.version, HashMap::new()));
    if *version != current.version {
        *version = current.version;
        formatters.clear();
    }
    formatters
        .entry(language)
        .or_insert_with(|| Arc::new(current.formatter(language)))
        .clone()
}

/// `language` and what it falls back to, short of `und`, in the order formatjs negotiates them.
fn fallback_chain(language: Language) -> Vec<Language> {
    static FALLBACKER: LazyLock<LocaleFallbacker> = LazyLock::new(|| Data::builtin().fallbacker());
    let mut step = FALLBACKER
        .for_config(LocaleFallbackConfig::default())
        .fallback_for(LanguageIdentifier::from(language).into());
    let mut chain = Vec::new();
    while !step.get().is_unknown() {
        chain.push(Language::from(step.get().into_locale().id));
        step.step();
    }
    chain
}

/// The directories of the packs `language`'s pack layers over, farthest first.
fn layers(dirs: &BTreeMap<Language, Vec<PathBuf>>, language: Language) -> Vec<PathBuf> {
    fallback_chain(language)
        .iter()
        .skip(1)
        .rev()
        .filter_map(|step| dirs.get(step))
        .flatten()
        .cloned()
        .collect()
}

/// The first step of `language`'s fallback chain, which its pack is filed under:
/// `zh-Hans` is `zh` and `zh-Hans-CN` is `zh-CN`, while `zh-Hant` and `en-US` stay as they are.
/// Compare a setting with [`Translations::languages`] through this.
#[must_use]
pub fn pack_language(language: Language) -> Language {
    fallback_chain(language)
        .first()
        .copied()
        .unwrap_or(language)
}

/// The translations every [`Translations::shared`] hands out; `None` until the first use.
static CURRENT: RwLock<Option<Arc<Translations>>> = RwLock::new(None);

/// Held by every test that puts translations in effect for the whole process,
/// as `cargo test` runs tests as threads of one process.
#[cfg(test)]
static PROCESS_WIDE: Mutex<()> = Mutex::new(());

/// The packs' facts, read up front, and their translations and CLDR data, read on first use.
pub struct Translations {
    /// Keyed by [`pack_language`].
    packs: BTreeMap<Language, Pack>,
    presentation: packs::Presentation,
    facts_report: LoadReport,
    /// For a language without a pack.
    english: OnceLock<Formatting>,
    version: u64,
}

impl fmt::Debug for Translations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Translations")
            .field("version", &self.version)
            .field("packs", &self.packs.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

struct Pack {
    dirs: Vec<PathBuf>,
    layers: Vec<PathBuf>,
    loaded: OnceLock<Loaded>,
    #[cfg(test)]
    loads: std::sync::atomic::AtomicUsize,
}

struct Loaded {
    report: LoadReport,
    /// `None` when the pack was refused.
    contents: Option<PackContents>,
}

struct PackContents {
    formatting: Formatting,
    /// The web UI's; the device serves them, the browser formats them.
    web: Option<Messages>,
    /// By widget uid.
    widgets: BTreeMap<String, Messages>,
    units: Option<units::Loaded>,
}

/// What formatters over a catalog share: CLDR data and the parse cache.
struct Formatting {
    catalog: Arc<MessageCatalog>,
    context: IntlContext,
}

impl Formatting {
    fn new(mut catalog: MessageCatalog, context: IntlContext) -> Self {
        catalog
            .insert(DEFAULT_LOCALE, HashMap::new())
            .expect("BUG: the default locale is a valid BCP-47 tag");
        Self {
            catalog: Arc::new(catalog),
            context,
        }
    }
}

impl Pack {
    fn new(dirs: Vec<PathBuf>, layers: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            layers,
            loaded: OnceLock::new(),
            #[cfg(test)]
            loads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Loads on the first call.
    fn loaded(&self, language: Language, log: bool) -> &Loaded {
        self.loaded.get_or_init(|| {
            #[cfg(test)]
            self.loads.fetch_add(1, Ordering::Relaxed);
            let loaded = load_pack(language, &self.layers, &self.dirs);
            if log {
                log_report(&format!("language pack {language} loaded"), &loaded.report);
            }
            loaded
        })
    }

    /// Whether a load has run and refused the pack; until one runs, only the facts speak for it.
    fn refused(&self) -> bool {
        self.loaded
            .get()
            .is_some_and(|loaded| loaded.contents.is_none())
    }
}

/// Refused when its CLDR data lacks its own locale, as it would format with root data.
fn load_pack(language: Language, layers: &[PathBuf], dirs: &[PathBuf]) -> Loaded {
    let mut report = LoadReport::default();
    let contents = packs::read_contents(layers, dirs, &mut report);
    // Over the base data and this pack's blob alone:
    // a pack carries its own language's data, as it must on a device that holds no other pack.
    let context = Data::with_packs(
        contents
            .blob
            .into_iter()
            .map(|blob| (language, blob))
            .collect(),
    )
    .context();

    let mut catalog = MessageCatalog::new();
    let checked = catalog
        .insert(language.tag(), contents.system)
        .and_then(|()| context.check_catalog(&catalog));
    if let Err(error) = checked {
        report.refused.push((language, error.to_string()));
        for file in &contents.own_files {
            report
                .skipped
                .push(format!("{file} in {language}: the language is not offered"));
        }
        return Loaded {
            report,
            contents: None,
        };
    }
    report.languages.push(language);
    Loaded {
        report,
        contents: Some(PackContents {
            formatting: Formatting::new(catalog, context),
            web: contents.web,
            widgets: contents.widgets,
            units: contents.units,
        }),
    }
}

fn log_report(what: &str, report: &LoadReport) {
    if report.is_clean() {
        tracing::info!("{what}: {report}");
    } else {
        tracing::warn!("{what}: {report}");
    }
}

fn next_version() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Translations {
    /// The packs under `roots`, only their facts read; the rest loads on first use.
    #[must_use]
    pub fn read(roots: &[PathBuf]) -> (Self, LoadReport) {
        let mut report = LoadReport::default();
        let facts = packs::read(roots, &mut report);
        report.languages = facts.dirs.keys().copied().collect();
        let translations = Self {
            packs: facts
                .dirs
                .iter()
                .map(|(&language, dirs)| {
                    (
                        language,
                        Pack::new(dirs.clone(), layers(&facts.text_dirs, language)),
                    )
                })
                .collect(),
            presentation: facts.presentation,
            facts_report: report.clone(),
            english: OnceLock::new(),
            version: next_version(),
        };
        (translations, report)
    }

    /// Load every pack now, leaving out each the load refuses.
    #[must_use]
    pub fn load_all(mut self) -> (Self, LoadReport) {
        let mut report = LoadReport {
            languages: Vec::new(),
            ..self.facts_report.clone()
        };
        let mut refused = Vec::new();
        for (language, pack) in &self.packs {
            let loaded = pack.loaded(*language, false);
            report.extend(&loaded.report);
            if loaded.contents.is_none() {
                refused.push(*language);
            }
        }
        for language in refused {
            self.packs.remove(&language);
            self.presentation.remove(language);
        }
        (self, report)
    }

    #[must_use]
    pub fn load(roots: &[PathBuf]) -> (Self, LoadReport) {
        Self::read(roots).0.load_all()
    }

    /// The translations in effect, until [`Self::reload`] or [`Self::install`].
    /// The first call [`Self::read`]s [`packs::roots`];
    /// install [`Self::load_all`]'s to check every pack up front.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        if let Some(current) = CURRENT
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            return current.clone();
        }
        CURRENT
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| {
                let (translations, report) = Self::read(&packs::roots());
                log_report(
                    "language packs named, each checked when first used",
                    &report,
                );
                Arc::new(translations)
            })
            .clone()
    }

    /// Load [`packs::roots`] afresh and put them in effect;
    /// a formatter already built keeps the translations it was built over.
    #[must_use = "the report says what the load refused and skipped"]
    pub fn reload() -> LoadReport {
        let (fresh, report) = Self::load(&packs::roots());
        Self::install(fresh);
        report
    }

    pub fn install(translations: Self) {
        *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(translations));
    }

    /// Higher for a later load.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// English, then the packs'.
    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        std::iter::once(Language::EN)
            .chain(
                self.packs
                    .iter()
                    .filter(|(_, pack)| !pack.refused())
                    .map(|(language, _)| *language),
            )
            .collect()
    }

    /// The first pack along `language`'s fallback chain no load has refused.
    fn pack_for(&self, language: Language) -> Option<Language> {
        fallback_chain(language)
            .into_iter()
            .find(|step| self.packs.get(step).is_some_and(|pack| !pack.refused()))
    }

    /// The first pack along `language`'s fallback chain the load does not refuse,
    /// as negotiation passes a language with no catalog.
    fn contents_for(&self, language: Language) -> Option<&PackContents> {
        fallback_chain(language)
            .into_iter()
            .find_map(|step| self.packs.get(&step)?.loaded(step, true).contents.as_ref())
    }

    /// The `info.json` of the pack it resolves to, else the tag as the name.
    #[must_use]
    pub fn info(&self, language: Language) -> LanguageInfo {
        self.pack_for(language)
            .and_then(|pack| self.presentation.info.get(&pack).cloned())
            .unwrap_or_else(|| LanguageInfo {
                // No pack is ever filed under English.
                native_name: if language == Language::EN {
                    "English".to_owned()
                } else {
                    language.tag()
                },
                text_direction: TextDirection::Ltr,
            })
    }

    #[must_use]
    pub fn web(&self, language: Language) -> Option<Messages> {
        self.contents_for(language)?.web.clone()
    }

    #[must_use]
    pub fn widget(&self, uid: &str, language: Language) -> Option<Messages> {
        self.contents_for(language)?.widgets.get(uid).cloned()
    }

    /// At most [`packs::MAX_FLAG_BYTES`] long.
    #[must_use]
    pub fn flag_svg(&self, language: Language) -> Option<&Path> {
        self.flag(&self.presentation.flags_svg, language)
    }

    /// The flag compiled to the icon format the device draws.
    #[must_use]
    pub fn flag_bin(&self, language: Language) -> Option<&Path> {
        self.flag(&self.presentation.flags_bin, language)
    }

    /// The flag of the pack it resolves to: a flag names a region, so a parent's never fills in.
    fn flag<'a>(
        &self,
        flags: &'a BTreeMap<Language, PathBuf>,
        language: Language,
    ) -> Option<&'a Path> {
        flags.get(&self.pack_for(language)?).map(PathBuf::as_path)
    }

    #[must_use]
    pub fn units(&self, language: Language) -> Option<units::Loaded> {
        self.contents_for(language)?.units.clone()
    }

    #[must_use]
    pub fn formatter(&self, language: Language) -> Intl {
        let formatting = self.contents_for(language).map_or_else(
            || {
                self.english.get_or_init(|| {
                    Formatting::new(MessageCatalog::new(), Data::builtin().context())
                })
            },
            |contents| &contents.formatting,
        );
        // FormatJS's third refusal, a poisoned cache, takes a precompiled catalog; none here is.
        Intl::over(language, formatting.catalog.clone(), &formatting.context)
            .expect("BUG: `en` is in every catalog and a Language's tag parses")
    }

    /// How many times `language`'s pack has loaded.
    #[cfg(test)]
    fn loads(&self, language: Language) -> usize {
        self.packs[&language].loads.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests;
