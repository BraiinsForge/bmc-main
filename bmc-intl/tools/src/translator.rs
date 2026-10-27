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

//! Translator files: what `formatjs extract` finds, what a translator
//! file holds, and the rules that keep the one in step with the other.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::console;
use crate::surface::parents_of;

/// A message as `formatjs extract --format crowdin` writes it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Extracted {
    pub message: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A translator file's entry: the English,
/// its translation or `None`, the translator's context.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub source: String,
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub type Extraction = BTreeMap<String, Extracted>;
pub type Entries = BTreeMap<String, Entry>;

#[cfg(test)]
impl Extracted {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
            description: None,
        }
    }
}

#[cfg(test)]
impl Entry {
    pub fn new(source: &str, translation: Option<&str>) -> Self {
        Self {
            source: source.to_owned(),
            translation: translation.map(str::to_owned),
            description: None,
        }
    }
}

/// A translator file's entries; none when there is no such file.
///
/// # Errors
/// The file named, when it cannot be read or is not a translator file.
pub fn read_entries(path: &Path) -> Result<Entries, String> {
    match std::fs::read(path) {
        Ok(bytes) => decode_entries(&bytes, &console::path(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Entries::new()),
        Err(error) => Err(console::at(path, error)),
    }
}

/// Translator entries from `bytes`, read from `origin`.
///
/// # Errors
/// `origin` named, when the bytes are not a translator file.
pub fn decode_entries(bytes: &[u8], origin: &str) -> Result<Entries, String> {
    serde_json::from_slice(bytes)
        .map_err(|error| format!("{origin}: not a translator file: {error}"))
}

/// Write `data` as `intl` writes every JSON file: two-space indented,
/// non-ASCII as itself, a newline at the end.
///
/// # Errors
/// The path named, when it cannot be written.
pub fn write_json(path: &Path, data: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| console::at(parent, error))?;
    }
    let mut text =
        serde_json::to_string_pretty(data).expect("BUG: what is written here has string keys");
    text.push('\n');
    std::fs::write(path, text).map_err(|error| console::at(path, error))
}

/// One surface's translator entries, by pack language.
pub type ByLanguage = BTreeMap<String, Entries>;

/// `language`'s entries still waiting for a translator: untranslated in its own file
/// and in every pack it layers over, as the device shows a parent's words in their place.
pub fn untranslated<'e>(
    language: &str,
    by_language: &'e ByLanguage,
) -> impl Iterator<Item = (&'e String, &'e Entry)> {
    let beneath: Vec<&Entries> = parents_of(language)
        .iter()
        .filter_map(|parent| by_language.get(parent))
        .collect();
    let translated_beneath = move |id: &str| {
        beneath.iter().any(|entries| {
            entries
                .get(id)
                .is_some_and(|entry| entry.translation.is_some())
        })
    };
    by_language
        .get(language)
        .into_iter()
        .flatten()
        .filter(move |(id, entry)| entry.translation.is_none() && !translated_beneath(id))
}

/// Translator entries for `extracted`, carrying over translations from `prior`.
///
/// By id first; else by identical English when exactly one gone translated entry
/// and exactly one new message have it, so a message
/// whose description changed — and with it its id — keeps its words,
/// while a new message sharing a live one's English is left for a translator,
/// as are two new messages that split one gone message's English.
#[must_use]
pub fn merge(extracted: &Extraction, prior: &Entries) -> Entries {
    let mut by_source: HashMap<&str, Vec<&Entry>> = HashMap::new();
    for (id, entry) in prior {
        if !extracted.contains_key(id) {
            by_source.entry(&entry.source).or_default().push(entry);
        }
    }
    let mut new_by_source: HashMap<&str, usize> = HashMap::new();
    for (id, message) in extracted {
        if !prior.contains_key(id) {
            *new_by_source.entry(&message.message).or_default() += 1;
        }
    }

    extracted
        .iter()
        .map(|(id, message)| {
            let translation = if let Some(entry) = prior.get(id) {
                entry.translation.clone()
            } else {
                let same_source: Vec<&Entry> = by_source
                    .get(message.message.as_str())
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|entry| entry.translation.as_deref().is_some_and(|t| !t.is_empty()))
                    .collect();
                if let [only] = same_source[..]
                    && new_by_source.get(message.message.as_str()) == Some(&1)
                {
                    only.translation.clone()
                } else {
                    None
                }
            };
            let entry = Entry {
                source: message.message.clone(),
                translation,
                description: message.description.clone().filter(|d| !d.is_empty()),
            };
            (id.clone(), entry)
        })
        .collect()
}

/// How a translator file lags the extraction: ids, then `source`
/// and `description`, each naming `fix_command`, which brings it up to date.
#[must_use]
pub fn stale_entries(extracted: &Extraction, entries: &Entries, fix_command: &str) -> Vec<String> {
    let fix = format!("run {fix_command}");
    let missing: Vec<(&String, &Extracted)> = extracted
        .iter()
        .filter(|(id, _)| !entries.contains_key(*id))
        .collect();
    let gone: Vec<&String> = entries
        .keys()
        .filter(|id| !extracted.contains_key(*id))
        .collect();
    let mut reworded = Vec::new();
    let mut redescribed = Vec::new();
    for (id, expected) in extracted {
        let Some(actual) = entries.get(id) else {
            continue;
        };
        if actual.source != expected.message {
            reworded.push(format!(
                "{id}: source is {:?}, the code says {:?}; {fix}, then check the translation still fits",
                actual.source, expected.message
            ));
        }
        if actual.description.as_deref().unwrap_or_default()
            != expected.description.as_deref().unwrap_or_default()
        {
            redescribed.push(id);
        }
    }

    let mut found = Vec::new();
    found.extend(named_or_counted(
        &missing,
        |(id, message)| {
            format!(
                "missing entry {id} ({:?}); {fix} to add it",
                message.message
            )
        },
        |count| format!("{count} entries missing; {fix} to add them"),
    ));
    found.extend(named_or_counted(
        &gone,
        |id| format!("entry {id} no longer in the source; {fix} to drop it"),
        |count| format!("{count} entries no longer in the source; {fix} to drop them"),
    ));
    found.extend(reworded);
    found.extend(named_or_counted(
        &redescribed,
        |id| format!("{id}: description is stale; {fix}"),
        |count| format!("{count} descriptions stale; {fix}"),
    ));
    found
}

/// One of a kind named; a run of them only says the file was not extracted, so it is counted.
fn named_or_counted<T>(
    items: &[T],
    one: impl FnOnce(&T) -> String,
    many: impl FnOnce(usize) -> String,
) -> Option<String> {
    match items {
        [] => None,
        [item] => Some(one(item)),
        _ => Some(many(items.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extracted() -> Extraction {
        Extraction::from([
            (
                "aaa".to_owned(),
                Extracted {
                    message: "Clear".to_owned(),
                    description: Some("Weather condition".to_owned()),
                },
            ),
            ("bbb".to_owned(), Extracted::new("Loading…")),
        ])
    }

    #[test]
    fn merge_keeps_translations_by_id_and_nulls_new_ones() {
        let prior = Entries::from([(
            "aaa".to_owned(),
            Entry {
                description: Some("Weather condition".to_owned()),
                ..Entry::new("Clear", Some("晴"))
            },
        )]);
        assert_eq!(
            merge(&extracted(), &prior),
            Entries::from([
                (
                    "aaa".to_owned(),
                    Entry {
                        description: Some("Weather condition".to_owned()),
                        ..Entry::new("Clear", Some("晴"))
                    }
                ),
                ("bbb".to_owned(), Entry::new("Loading…", None)),
            ])
        );
    }

    #[test]
    fn merge_recovers_a_translation_by_identical_english_when_the_id_changed() {
        let prior = Entries::from([("old".to_owned(), Entry::new("Clear", Some("晴")))]);
        assert_eq!(
            merge(&extracted(), &prior)["aaa"].translation.as_deref(),
            Some("晴")
        );
    }

    #[test]
    fn merge_does_not_guess_between_two_prior_entries_with_the_same_english() {
        let prior = Entries::from([
            ("one".to_owned(), Entry::new("Clear", Some("晴"))),
            ("two".to_owned(), Entry::new("Clear", Some("清除"))),
        ]);
        assert_eq!(merge(&extracted(), &prior)["aaa"].translation, None);
    }

    #[test]
    fn merge_does_not_borrow_the_translation_of_a_message_still_extracted() {
        let extracted = Extraction::from([
            (
                "sky".to_owned(),
                Extracted {
                    message: "Clear".to_owned(),
                    description: Some("Weather condition".to_owned()),
                },
            ),
            (
                "reset".to_owned(),
                Extracted {
                    message: "Clear".to_owned(),
                    description: Some("Button that empties a form".to_owned()),
                },
            ),
        ]);
        let prior = Entries::from([("sky".to_owned(), Entry::new("Clear", Some("晴")))]);
        assert_eq!(merge(&extracted, &prior)["reset"].translation, None);
    }

    #[test]
    fn merge_does_not_hand_one_gone_translation_to_two_new_messages() {
        let extracted = Extraction::from([
            ("sky".to_owned(), Extracted::new("Clear")),
            ("reset".to_owned(), Extracted::new("Clear")),
        ]);
        let prior = Entries::from([("old".to_owned(), Entry::new("Clear", Some("晴")))]);
        let merged = merge(&extracted, &prior);
        assert_eq!(
            [
                merged["sky"].translation.clone(),
                merged["reset"].translation.clone()
            ],
            [None, None]
        );
    }

    #[test]
    fn merge_drops_entries_whose_message_is_gone() {
        let prior = Entries::from([("gone".to_owned(), Entry::new("Bye", Some("再见")))]);
        assert!(!merge(&extracted(), &prior).contains_key("gone"));
    }

    #[test]
    fn stale_entries_names_every_way_a_translator_file_can_lag() {
        let entries = Entries::from([
            (
                "aaa".to_owned(),
                Entry {
                    description: Some("Old context".to_owned()),
                    ..Entry::new("Clear skies", Some("晴"))
                },
            ),
            ("zzz".to_owned(), Entry::new("Removed", None)),
        ]);
        assert_eq!(
            stale_entries(&extracted(), &entries, "`fix`"),
            [
                "missing entry bbb (\"Loading…\"); run `fix` to add it",
                "entry zzz no longer in the source; run `fix` to drop it",
                "aaa: source is \"Clear skies\", the code says \"Clear\"; run `fix`, then check the translation still fits",
                "aaa: description is stale; run `fix`",
            ]
        );
    }

    #[test]
    fn stale_entries_counts_a_run_of_one_kind_instead_of_listing_it() {
        let entries = Entries::from([
            ("old1".to_owned(), Entry::new("One", None)),
            ("old2".to_owned(), Entry::new("Two", None)),
        ]);
        assert_eq!(
            stale_entries(&extracted(), &entries, "`fix`"),
            [
                "2 entries missing; run `fix` to add them",
                "2 entries no longer in the source; run `fix` to drop them",
            ]
        );
    }

    #[test]
    fn stale_entries_is_silent_for_a_current_file() {
        let extracted = extracted();
        assert!(stale_entries(&extracted, &merge(&extracted, &Entries::new()), "fix").is_empty());
    }

    #[test]
    fn a_written_file_reads_back_as_it_was_written() {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let path = dir.path().join("lang/zh.json");
        let entries = merge(&extracted(), &Entries::new());
        write_json(&path, &entries).expect("BUG: a writable temporary directory");
        assert_eq!(read_entries(&path), Ok(entries));
        let text = std::fs::read_to_string(&path).expect("BUG: just written");
        assert!(text.contains("\"Loading…\""), "non-ASCII as itself: {text}");
        assert!(text.ends_with("}\n"), "a newline at the end");
    }
}
