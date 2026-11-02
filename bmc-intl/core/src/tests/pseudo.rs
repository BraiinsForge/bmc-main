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

use formatjs_icu_messageformat::{Value, Values};
use formatjs_intl::MessageDescriptorRef;
use serde::Deserialize;

use super::{Root, base_blob, translator_file};
use crate::{Intl, Language, Translations, runs};

#[derive(Deserialize)]
struct Sample {
    source: String,
    values: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    tags: Vec<String>,
    spoken: String,
}

fn samples() -> Vec<Sample> {
    #[derive(Deserialize)]
    struct Samples {
        samples: Vec<Sample>,
    }
    serde_json::from_str::<Samples>(include_str!("../../../../assets/intl/pseudo-locale.json"))
        .expect("BUG: the pseudo-locale descriptor lists samples")
        .samples
}

/// An en-XA formatter loaded from a pack whose catalog holds `sources` respelled,
/// each filed under itself, as no test message reaches the extracted catalogs.
fn speaking(sources: &[&str]) -> Intl {
    let respelled: Vec<String> = sources
        .iter()
        .map(|source| bmc_intl_pseudo::respell(source))
        .collect();
    let entries: Vec<(&str, &str, Option<&str>)> = sources
        .iter()
        .zip(&respelled)
        .map(|(source, spoken)| (*source, *source, Some(spoken.as_str())))
        .collect();
    let root = Root::new();
    root.write_bytes("en-XA", "cldr.postcard", &base_blob())
        .write("en-XA", "system.json", &translator_file(&entries));
    let (translations, report) = Translations::load(&[root.path()]);
    assert!(report.is_clean(), "{report}");
    translations.formatter(Language::EN_XA)
}

#[test]
fn every_sample_is_spoken_as_the_descriptor_says() {
    let samples = samples();
    let sources: Vec<&str> = samples
        .iter()
        .map(|sample| sample.source.as_str())
        .collect();
    let intl = speaking(&sources);
    for sample in &samples {
        let mut values = Values::new();
        for (name, value) in &sample.values {
            let value = match value {
                serde_json::Value::Number(number) => Value::from(
                    number
                        .as_i64()
                        .expect("BUG: the samples' numbers are integers"),
                ),
                serde_json::Value::String(text) => Value::from(text.clone()),
                other @ (serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Array(_)
                | serde_json::Value::Object(_)) => {
                    panic!("BUG: a sample value is a number or a string, not {other}")
                }
            };
            values.insert(name.clone(), value);
        }
        for tag in &sample.tags {
            values.insert(tag.clone(), Value::tag(Ok));
        }
        let descriptor = MessageDescriptorRef::new(&sample.source, &sample.source);
        assert_eq!(
            intl.format_message_to_string_or_default_ref(descriptor, &values),
            sample.spoken,
            "{}",
            sample.source
        );
    }
}

#[test]
fn a_bold_stretch_keeps_the_brackets_outside_it() {
    let source = "Tap <b>here</b> to start";
    let intl = speaking(&[source]);
    let mut values = Values::new();
    values.insert("b".to_owned(), Value::tag(crate::bold));
    let text = intl.format_message_to_string_or_default_ref(
        MessageDescriptorRef::new(source, source),
        &values,
    );
    let runs = runs(&text);
    let spoken: Vec<(&str, bool)> = runs
        .iter()
        .map(|run| (run.text.as_str(), run.bold))
        .collect();
    assert_eq!(
        spoken,
        [("[Ŧȧȧƥ ", false), ("ħḗḗřḗḗ", true), (" ŧǿǿ şŧȧȧřŧ]", false)]
    );
}
