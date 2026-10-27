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

//! `intl cldr-units` and `intl show-units`: a language pack's `units.postcard`,
//! cut from the pinned cldr-json, and a readable dump of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

use bmc_intl_units::{
    Currency, Derivations, Derive, FILE, FORMAT, Map, Map2d, Plural, Spacing, Units, Width,
};
use bmc_shared_utils::language::Language;
use icu_locale::LocaleExpander;
use icu_locale_core::LanguageIdentifier;
use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde_json::Value;

use crate::console;

/// The device writes Latin digits in every language.
const MISC_PATTERNS: &str = "miscPatterns-numberSystem-latn";
const CURRENCY_FORMATS: &str = "currencyFormats-numberSystem-latn";

/// Write the pack's `units.postcard`, for the language its directory is named after.
///
/// # Errors
/// No pinned cldr-json, a pack not named by a language it has, or CLDR not shaped as expected.
pub fn cut(pack: &Path) -> Result<ExitCode, String> {
    let root = pinned()?;
    let path = pack.join(FILE);
    let release = read_json(&root.join("cldr-core/package.json"))?;
    let release = text_at(&release, &["version"])?;
    let units = units_of(&root, &pack_language(pack)?)?;
    std::fs::write(&path, units).map_err(|error| console::at(&path, error))?;
    console::ok(&format!(
        "{}: CLDR {}",
        console::path(&path),
        console::lit(release)
    ));
    Ok(ExitCode::SUCCESS)
}

/// Print the pack's `units.postcard` as JSON,
/// `pack` a language with a profile pack in `repo`, else a pack's directory.
///
/// # Errors
/// A file that cannot be read or is not units.
pub fn show(pack: &Path, repo: &Path) -> Result<ExitCode, String> {
    let profile = pack
        .to_str()
        .and_then(|key| Language::parse(key).ok())
        .map(|language| {
            repo.join(crate::surface::PROFILE_PACKS)
                .join(language.tag())
        })
        .filter(|dir| dir.is_dir());
    let pack = profile.unwrap_or_else(|| pack.to_path_buf());
    let path = pack.join(FILE);
    let bytes = std::fs::read(&path).map_err(|error| console::at(&path, error))?;
    let units = Units::from_bytes(&bytes).map_err(|error| console::at(&path, error))?;
    console::line(
        serde_json::to_string_pretty(&units).expect("BUG: units always serialize to JSON"),
    );
    Ok(ExitCode::SUCCESS)
}

/// The pinned cldr-json, which the dev shell names.
fn pinned() -> Result<PathBuf, String> {
    std::env::var_os("CLDR_JSON")
        .map(PathBuf::from)
        .ok_or_else(|| console::outside_dev_shell("CLDR_JSON is unset"))
}

/// The language a pack's directory is named after, as `bmc_intl` files it.
fn pack_language(pack: &Path) -> Result<String, String> {
    let resolved = pack
        .canonicalize()
        .map_err(|error| console::at(pack, error))?;
    resolved
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{} names no pack", console::path(pack)))
}

/// The cldr-json `main/` directory holding `language`, or `None` if CLDR has none:
/// the tag, or its ICU4X-maximised form (`zh-SG` is `zh-Hans-SG`),
/// less a region CLDR's default content folds into the parent (`pt-BR`),
/// then less a script that is the language's default (`pt-Latn` is `pt`).
fn cldr_locale(root: &Path, language: Language) -> Result<Option<String>, String> {
    static EXPANDER: LazyLock<LocaleExpander> = LazyLock::new(LocaleExpander::new_extended);
    let has = |name: &str| root.join(format!("cldr-units-full/main/{name}")).is_dir();
    let tag = language.tag();
    if has(&tag) {
        return Ok(Some(tag));
    }
    let mut maximised = LanguageIdentifier::from(language);
    EXPANDER.maximize(&mut maximised);
    let maximised_tag = maximised.to_string();
    if has(&maximised_tag) {
        return Ok(Some(maximised_tag));
    }
    let (primary, script) = (maximised.language, maximised.script);
    if let Some(region) = maximised.region {
        let defaults = read_json(&root.join("cldr-core/defaultContent.json"))?;
        let defaults = typed_at(&defaults, &["defaultContent"], Value::as_array, "a list")?;
        let in_parent = [(None, Some(region)), (script, Some(region))]
            .map(|(script, region)| LanguageIdentifier::from((primary, script, region)).to_string())
            .iter()
            .chain([&tag])
            .any(|form| defaults.iter().any(|default| default == form));
        if !in_parent {
            return Ok(None);
        }
    }
    let with_script = LanguageIdentifier::from((primary, script, None)).to_string();
    if has(&with_script) {
        return Ok(Some(with_script));
    }
    let mut own = LanguageIdentifier::from((primary, None, None));
    EXPANDER.maximize(&mut own);
    let primary = primary.to_string();
    Ok((own.script == script && has(&primary)).then_some(primary))
}

/// # Errors
/// A pack name that is no canonical language tag,
/// a language the pinned cldr-json lacks, or CLDR not shaped as expected.
fn units_of(root: &Path, language: &str) -> Result<Vec<u8>, String> {
    let parsed = Language::parse(language)
        .map_err(|error| format!("{}: not a language: {error}", console::refused(language)))?;
    if parsed.tag() != language {
        return Err(format!(
            "{}: spell it {}, the tag's canonical casing",
            console::refused(language),
            console::lit(&parsed.tag())
        ));
    }
    let primary = parsed.primary().tag();
    let cldr: &str = &cldr_locale(root, parsed)?.ok_or_else(|| {
        format!(
            "{}: no such language in the pinned cldr-json",
            console::refused(language)
        )
    })?;
    let units = read_json(&root.join(format!("cldr-units-full/main/{cldr}/units.json")))?;
    let numbers = read_json(&root.join(format!("cldr-numbers-full/main/{cldr}/numbers.json")))?;
    let ranges = read_json(&root.join("cldr-core/supplemental/pluralRanges.json"))?;
    let grammar = read_json(&root.join("cldr-core/supplemental/grammaticalFeatures.json"))?;

    let numbers = at(&numbers, &["main", cldr, "numbers"])?;
    // By truncation alone: `parentLocales` leaves its `plurals` and `grammaticalFeatures` empty.
    let ranges = [cldr, primary.as_str()]
        .into_iter()
        .map(|key| ["supplemental", "plurals", key])
        .find(|keys| at(&ranges, keys).is_ok())
        .map(|keys| object_at(&ranges, &keys))
        .transpose()?;
    let grammar = at(&grammar, &["supplemental", "grammaticalData"])?;

    Ok(Units {
        format: FORMAT,
        short: width(&units, cldr, "short")?,
        narrow: width(&units, cldr, "narrow")?,
        range: text_at(numbers, &[MISC_PATTERNS, "range"])?.into(),
        plural_ranges: plural_ranges(ranges)?.into_iter().collect(),
        derivations: derivations(grammar, &[cldr, primary.as_str(), "und"])?,
        currency: currency(numbers)?,
    }
    .to_bytes())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| console::at(path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| console::at(path, error))
}

/// `value[key]…` along `keys`, naming the path when the pinned CLDR does not hold it.
fn at<'v>(value: &'v Value, keys: &[&str]) -> Result<&'v Value, String> {
    keys.iter()
        .try_fold(value, |value, key| value.get(key))
        .ok_or_else(|| format!("the pinned cldr-json has no `{}`", keys.join(".")))
}

/// What is at `keys` as `cast` takes it, naming the path and `kind` when it is not that.
fn typed_at<'v, T: ?Sized>(
    value: &'v Value,
    keys: &[&str],
    cast: fn(&Value) -> Option<&T>,
    kind: &str,
) -> Result<&'v T, String> {
    cast(at(value, keys)?)
        .ok_or_else(|| format!("`{}` in the pinned cldr-json is not {kind}", keys.join(".")))
}

fn object_at<'v>(
    value: &'v Value,
    keys: &[&str],
) -> Result<&'v serde_json::Map<String, Value>, String> {
    typed_at(value, keys, Value::as_object, "an object")
}

fn text_at<'v>(value: &'v Value, keys: &[&str]) -> Result<&'v str, String> {
    typed_at(value, keys, Value::as_str, "a string")
}

/// A plural category as CLDR spells it, which is how [`Plural`] deserializes.
fn plural(name: &str) -> Result<Plural, String> {
    let name: StrDeserializer<'_, ValueError> = name.into_deserializer();
    Plural::deserialize(name).map_err(|error| error.to_string())
}

/// What a key in a unit's entry holds.
enum Key {
    Unit(Plural),
    PerUnit,
    Prefix,
    Compound,
    Power(Plural),
    /// `compoundUnitPattern1`, which only fills `other` where no counted pattern does.
    PowerUncounted,
    /// No pattern a figure is placed by: a unit's name alone, a compass point's.
    Skipped,
}

fn key(name: &str) -> Result<Key, String> {
    let unknown =
        || format!("`{name}` is a unit key `intl cldr-units` does not know: the pinned CLDR moved");
    if let Some(count) = name.strip_prefix("unitPattern-count-") {
        return plural(count).map(Key::Unit).map_err(|_| unknown());
    }
    if let Some(count) = name.strip_prefix("compoundUnitPattern1-count-") {
        return plural(count).map(Key::Power).map_err(|_| unknown());
    }
    match name {
        "perUnitPattern" => Ok(Key::PerUnit),
        "unitPrefixPattern" => Ok(Key::Prefix),
        "compoundUnitPattern" => Ok(Key::Compound),
        "compoundUnitPattern1" => Ok(Key::PowerUncounted),
        "displayName" | "east" | "north" | "south" | "west" => Ok(Key::Skipped),
        _ => Err(unknown()),
    }
}

fn width(units: &Value, language: &str, width: &'static str) -> Result<Width<'static>, String> {
    let keys = ["main", language, "units", width];
    let (mut counted, mut per_unit, mut prefixes, mut compounds) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut powers, mut uncounted_powers) = (BTreeMap::new(), Vec::new());
    for unit in object_at(units, &keys)?.keys() {
        let entry = [keys.as_slice(), &[unit.as_str()]].concat();
        let unit = unit.as_str();
        for name in object_at(units, &entry)?.keys() {
            let pattern = || text_at(units, &[entry.as_slice(), &[name.as_str()]].concat());
            match key(name)? {
                Key::Unit(plural) => counted.push((unit, plural, pattern()?)),
                Key::Power(plural) => drop(powers.insert((unit, plural), pattern()?)),
                Key::PerUnit => per_unit.push((unit, pattern()?)),
                Key::Prefix => prefixes.push((unit, pattern()?)),
                Key::Compound => compounds.push((unit, pattern()?)),
                Key::PowerUncounted => uncounted_powers.push((unit, pattern()?)),
                Key::Skipped => {}
            }
        }
    }
    for (unit, pattern) in uncounted_powers {
        powers.entry((unit, Plural::Other)).or_insert(pattern);
    }
    let built = |map: &'static str| move |why: &str| format!("`{width}.{map}`: {why}");
    Ok(Width {
        units: Map2d::new(counted).map_err(built("units"))?,
        per_unit: Map::new(per_unit).map_err(built("per_unit"))?,
        prefixes: Map::new(prefixes).map_err(built("prefixes"))?,
        compounds: Map::new(compounds).map_err(built("compounds"))?,
        powers: Map2d::new(
            powers
                .into_iter()
                .map(|((unit, plural), pattern)| (unit, plural, pattern)),
        )
        .map_err(built("powers"))?,
    })
}

/// `pluralRange-start-one-end-few: few`, as (one, few, few); none for a language without them.
fn plural_ranges(
    ranges: Option<&serde_json::Map<String, Value>>,
) -> Result<Vec<(Plural, Plural, Plural)>, String> {
    let mut found = Vec::new();
    for (name, result) in ranges.into_iter().flatten() {
        let (start, end) = name
            .strip_prefix("pluralRange-start-")
            .and_then(|rest| rest.split_once("-end-"))
            .ok_or_else(|| format!("`{name}` is no plural range"))?;
        let result = result
            .as_str()
            .ok_or_else(|| format!("`{name}` is not a plural category"))?;
        found.push((plural(start)?, plural(end)?, plural(result)?));
    }
    Ok(found)
}

/// Each structure's rule, from the first of `chain` that has it:
/// CLDR inherits a rule a language does not give from its parents, then from `und`.
fn derivations(grammar: &Value, chain: &[&str]) -> Result<Derivations, String> {
    let parts = |structure: &str| -> Result<[Derive; 2], String> {
        let name = format!("deriveComponent-feature-plural-structure-{structure}");
        let rule = chain
            .iter()
            .find_map(|language| grammar.get(language)?.get(&name))
            .ok_or_else(|| format!("the pinned cldr-json has no `{name}`, not even for `und`"))?;
        let part = |value: &str| -> Result<Derive, String> {
            match rule.get(value).and_then(Value::as_str) {
                Some("compound") => Ok(Derive::Compound),
                Some(fixed) => plural(fixed).map(Derive::Fixed),
                None => Err(format!("`{name}` has no `{value}`")),
            }
        };
        Ok([part("_value0")?, part("_value1")?])
    };
    Ok(Derivations {
        per: parts("per")?,
        times: parts("times")?,
        power: parts("power")?,
        prefix: parts("prefix")?,
    })
}

fn currency(numbers: &Value) -> Result<Currency<'_>, String> {
    let field = |keys: &[&str]| text_at(numbers, &[&[CURRENCY_FORMATS], keys].concat());
    let spacing = |side: &str| -> Result<Spacing<'_>, String> {
        let of = |name: &str| field(&["currencySpacing", side, name]);
        Ok(Spacing {
            currency_match: of("currencyMatch")?.into(),
            surrounding_match: of("surroundingMatch")?.into(),
            insert_between: of("insertBetween")?.into(),
        })
    };
    Ok(Currency {
        standard: field(&["standard"])?.into(),
        alpha_next_to_number: field(&["standard-alphaNextToNumber"])?.into(),
        before: spacing("beforeCurrency")?,
        after: spacing("afterCurrency")?,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn write(root: &Path, path: &str, value: &Value) {
        crate::translator::write_json(&root.join(path), value)
            .expect("BUG: a writable temporary directory");
    }

    /// A pinned cldr-json holding Czech, its width `short` as given, `narrow` empty.
    /// `cs` brings a gender rule alone, so its plural rules must come from `und`.
    fn cldr(short: &Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        write(
            root,
            "cldr-units-full/main/cs/units.json",
            &json!({ "main": { "cs": { "units": {
                "long": {},
                "short": short,
                "narrow": {},
                "durationUnit-type-hm": { "durationUnitPattern": "h:mm" },
            } } } }),
        );
        let spacing = json!({ "currencyMatch": "[:^S:]", "surroundingMatch": "[:digit:]", "insertBetween": " " });
        write(
            root,
            "cldr-numbers-full/main/cs/numbers.json",
            &json!({ "main": { "cs": { "numbers": {
                "miscPatterns-numberSystem-latn": { "range": "{0}–{1}" },
                "currencyFormats-numberSystem-latn": {
                    "standard": "#,##0.00 ¤",
                    "standard-alphaNextToNumber": "#,##0.00 ¤",
                    "currencySpacing": { "beforeCurrency": spacing, "afterCurrency": spacing },
                },
            } } } }),
        );
        write(
            root,
            "cldr-core/supplemental/pluralRanges.json",
            &json!({ "supplemental": { "plurals": { "cs": {
                "pluralRange-start-one-end-few": "few",
            } } } }),
        );
        let rule = |first: &str, second: &str| json!({ "_value0": first, "_value1": second });
        write(
            root,
            "cldr-core/supplemental/grammaticalFeatures.json",
            &json!({ "supplemental": { "grammaticalData": {
                "cs": { "deriveCompound-feature-gender-structure-per": "0" },
                "und": {
                    "deriveComponent-feature-plural-structure-per": rule("compound", "one"),
                    "deriveComponent-feature-plural-structure-times": rule("one", "compound"),
                    "deriveComponent-feature-plural-structure-power": rule("one", "compound"),
                    "deriveComponent-feature-plural-structure-prefix": rule("one", "compound"),
                },
            } } }),
        );
        dir
    }

    fn czech_short() -> Value {
        json!({
            "temperature-celsius": {
                "displayName": "°C",
                "unitPattern-count-one": "{0} °C",
                "unitPattern-count-other": "{0} °C",
            },
            "duration-hour": { "unitPattern-count-other": "{0} h", "perUnitPattern": "{0}/h" },
            "10p9": { "unitPrefixPattern": "G{0}" },
            "per": { "compoundUnitPattern": "{0}/{1}" },
            "power2": { "compoundUnitPattern1": "{0}^2", "compoundUnitPattern1-count-other": "{0}²" },
            "power3": { "compoundUnitPattern1": "{0}³" },
            "coordinateUnit": { "displayName": "směr", "east": "{0} v. d." },
        })
    }

    /// Why `units_of` refuses `language` over the fixture.
    fn refusal(short: &Value, language: &str) -> String {
        let dir = cldr(short);
        units_of(dir.path(), language).expect_err("BUG: the case is one `units_of` refuses")
    }

    #[test]
    fn each_cldr_pattern_lands_where_its_key_says() {
        let dir = cldr(&czech_short());
        let bytes = units_of(dir.path(), "cs").expect("BUG: Czech is in the fixture");
        let units = Units::from_bytes(&bytes).expect("BUG: just written");
        let short = &units.short;
        assert_eq!(
            short.units.get("temperature-celsius", Plural::One),
            Some("{0} °C")
        );
        assert_eq!(short.per_unit.get("duration-hour"), Some("{0}/h"));
        assert_eq!(short.prefixes.get("10p9"), Some("G{0}"));
        assert_eq!(short.compounds.get("per"), Some("{0}/{1}"));
        assert_eq!(
            short.powers.get("power2", Plural::Other),
            Some("{0}²"),
            "a counted pattern wins over the uncounted one"
        );
        assert_eq!(
            short.powers.get("power3", Plural::Other),
            Some("{0}³"),
            "the uncounted pattern fills `other` where nothing counted does"
        );
        assert_eq!(
            short.units.get("coordinateUnit", Plural::Other),
            None,
            "a compass point places no figure"
        );
        assert_eq!(units.range, "{0}–{1}");
        assert_eq!(
            units.plural_range(Plural::One, Plural::Few),
            Some(Plural::Few)
        );
        assert_eq!(
            units.derivations.per,
            [Derive::Compound, Derive::Fixed(Plural::One)],
            "a rule Czech does not give is `und`'s"
        );
        assert_eq!(units.currency.standard, "#,##0.00 ¤");
        assert_eq!(units.currency.alpha_next_to_number, "#,##0.00 ¤");
    }

    #[test]
    fn a_unit_key_cldr_did_not_have_is_refused_by_name() {
        let refused = refusal(
            &json!({ "duration-hour": { "unitPattern-count-other-case-genitive": "{0} h" } }),
            "cs",
        );
        assert!(
            refused.contains("`unitPattern-count-other-case-genitive`"),
            "{refused}"
        );
    }

    #[test]
    fn a_pack_resolves_to_the_cldr_directory_holding_its_language() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let root = dir.path();
        for name in ["de-AT", "de", "zh-Hans-SG", "zh-Hant", "pt", "cs", "en"] {
            std::fs::create_dir_all(root.join("cldr-units-full/main").join(name))
                .expect("BUG: a writable temporary directory");
        }
        write(
            root,
            "cldr-core/defaultContent.json",
            &json!({ "defaultContent": ["pt-BR", "cs-CZ", "zh-Hant-TW", "de-DE", "en-US"] }),
        );
        let found = |tag: &str| {
            let language = Language::parse(tag).expect("BUG: a fixture tag");
            cldr_locale(root, language).expect("BUG: the fixture reads")
        };
        for (pack, cldr) in [
            ("de-AT", "de-AT"),
            ("zh-SG", "zh-Hans-SG"),
            ("pt-BR", "pt"),
            ("cs-CZ", "cs"),
            ("zh-Hant-TW", "zh-Hant"),
            ("pt-Latn", "pt"),
        ] {
            assert_eq!(found(pack).as_deref(), Some(cldr), "{pack}");
        }
        assert_eq!(
            found("de-CH"),
            None,
            "a region outside CLDR's default content does not fall back to its parent"
        );
        assert_eq!(
            found("en-Cyrl"),
            None,
            "a script other than the language's own does not fall back to it"
        );
    }

    #[test]
    fn a_language_the_pinned_cldr_lacks_is_refused() {
        let refused = refusal(&czech_short(), "tlh");
        assert!(refused.contains("no such language"), "{refused}");
    }

    #[test]
    fn a_pack_named_by_no_language_is_refused() {
        let refused = refusal(&czech_short(), "cs_CZ");
        assert!(refused.contains("not a language"), "{refused}");
    }

    #[test]
    fn a_pack_named_in_another_casing_is_told_its_name() {
        let refused = refusal(&czech_short(), "CS");
        assert!(refused.contains("canonical casing"), "{refused}");
    }
}
