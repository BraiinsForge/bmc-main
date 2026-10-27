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

use std::path::{Path, PathBuf};

use formatjs_icu_messageformat::Locale;

use super::*;

type R<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The firmware's own packs: Chinese, with its CLDR data and no translations yet.
fn profile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/intl/profile-languages")
}

/// The profile's Chinese pack's `file`.
fn profile_zh(file: &str) -> Vec<u8> {
    std::fs::read(profile().join("zh").join(file))
        .expect("BUG: the profile's Chinese pack holds it")
}

/// A blob that opens but holds English and root alone: the facts pass a pack with it, its load does not.
fn base_blob() -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/intl/cldr-base.postcard"),
    )
    .expect("BUG: the checkout's base blob")
}

const ES_MX_FIXTURE: &str = "fixtures/es-MX/cldr.postcard";

/// `es-MX`'s own blob, for a chain of three; `just intl::bump-icu4x` re-cuts it.
fn es_mx_blob() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(ES_MX_FIXTURE))
        .expect("BUG: the checkout's es-MX fixture")
}

/// What git leaves of a git-lfs file in a checkout that never pulled it.
const LFS_POINTER_FILE: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:0\nsize 1\n";

/// A root holding packs written by the test.
struct Root(tempfile::TempDir);

impl Root {
    fn new() -> Self {
        Self(tempfile::tempdir().expect("BUG: a temporary directory"))
    }

    fn path(&self) -> PathBuf {
        self.0.path().to_path_buf()
    }

    /// Write `contents` to `file` in the pack `key`.
    fn write(&self, key: &str, file: &str, contents: &str) -> &Self {
        self.write_bytes(key, file, contents.as_bytes())
    }

    fn write_bytes(&self, key: &str, file: &str, contents: &[u8]) -> &Self {
        let path = self.0.path().join(key).join(file);
        std::fs::create_dir_all(path.parent().expect("BUG: a file in a pack"))
            .expect("BUG: a writable temporary directory");
        std::fs::write(path, contents).expect("BUG: a writable temporary directory");
        self
    }

    /// A pack directory with nothing in it.
    fn language(&self, key: &str) -> &Self {
        std::fs::create_dir_all(self.0.path().join(key))
            .expect("BUG: a writable temporary directory");
        self
    }
}

/// A translator file of `(id, source, translation)` entries.
fn translator_file(entries: &[(&str, &str, Option<&str>)]) -> String {
    let entries: serde_json::Map<String, serde_json::Value> = entries
        .iter()
        .map(|(id, source, translation)| {
            (
                (*id).to_owned(),
                serde_json::json!({ "source": source, "translation": translation }),
            )
        })
        .collect();
    serde_json::Value::Object(entries).to_string()
}

/// What the load left out, one finding a line.
fn skipped(report: &LoadReport) -> String {
    report.skipped.join("\n")
}

#[test]
fn a_message_speaks_english_with_no_pack() {
    let (translations, report) = Translations::load(&[]);
    assert!(report.is_clean(), "{report}");
    let intl = translations.formatter(Language::ZH);
    let text = format_message!(intl, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
    assert_eq!(text, "Connect to deck");
}

#[test]
fn a_message_speaks_its_translation_once_its_pack_is_loaded() {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[("probe", "Connect to {ssid}", Some("连接到 {ssid}"))]),
    );
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    assert!(report.is_clean(), "{report}");

    let zh = translations.formatter(Language::ZH);
    let text = format_message!(zh, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
    assert_eq!(text, "连接到 deck");

    let en = translations.formatter(Language::EN);
    let text = format_message!(en, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
    assert_eq!(text, "Connect to deck");
}

/// The catalog formatjs picks for `requested` when English and `offered` are on offer.
fn negotiated(requested: &str, offered: &str) -> R<String> {
    let mut catalog = MessageCatalog::new();
    catalog.insert(DEFAULT_LOCALE, HashMap::new())?;
    catalog.insert(offered, HashMap::new())?;
    let default_locale: Locale = DEFAULT_LOCALE.parse()?;
    Ok(Data::builtin()
        .context()
        .negotiate_locale([requested], &default_locale, &catalog)?
        .to_string())
}

#[test]
fn a_language_negotiates_to_the_pack_it_is_filed_under() -> R {
    for (requested, filed) in [
        ("zh", "zh"),
        ("zh-Hans", "zh"),
        ("sr-Cyrl", "sr"),
        ("sr-Latn", "sr-Latn"),
        ("zh-Hant", "zh-Hant"),
        ("en-US", "en-US"),
        ("en-GB", "en-GB"),
        ("pt-BR", "pt-BR"),
        ("pt-PT", "pt-PT"),
    ] {
        let language = Language::parse(requested)?;
        assert_eq!(pack_language(language).tag(), filed, "{requested}'s pack");
        assert_eq!(
            negotiated(requested, filed)?,
            filed,
            "{requested} reaches its pack"
        );
    }
    Ok(())
}

#[test]
fn a_catalog_under_another_spelling_is_never_reached() -> R {
    assert_eq!(negotiated("zh-Hans", "zh-Hans")?, DEFAULT_LOCALE);
    assert_eq!(negotiated("zh-Hant", "zh-TW")?, DEFAULT_LOCALE);
    Ok(())
}

#[test]
fn a_later_root_overrides_an_earlier_one_entry_by_entry() {
    let (earlier, later) = (Root::new(), Root::new());
    earlier.write(
        "zh",
        "system.json",
        &translator_file(&[("kept", "Kept", Some("保留")), ("over", "Over", Some("旧"))]),
    );
    later.write(
        "zh",
        "system.json",
        &translator_file(&[("over", "Over", Some("新"))]),
    );
    let (translations, report) = Translations::load(&[profile(), earlier.path(), later.path()]);
    assert!(report.is_clean(), "{report}");
    let zh = translations.formatter(Language::ZH);
    let kept = format_message!(zh, id: "kept", default_message: "Kept");
    let over = format_message!(zh, id: "over", default_message: "Over");
    assert_eq!((kept.as_str(), over.as_str()), ("保留", "新"));
}

#[test]
fn the_offered_languages_are_english_then_every_accepted_pack() {
    let (translations, report) = Translations::load(&[profile()]);
    assert!(report.is_clean(), "{report}");
    assert_eq!(report.languages, [Language::ZH]);
    assert_eq!(translations.languages(), [Language::EN, Language::ZH]);
    assert_eq!(translations.info(Language::ZH).native_name, "简体中文");
    assert_eq!(translations.info(Language::EN).native_name, "English");
    assert!(
        translations.units(Language::ZH).is_some(),
        "the profile pack's unit patterns"
    );
    assert!(
        translations.flag_svg(Language::ZH).is_some()
            && translations.flag_bin(Language::ZH).is_some(),
        "the profile pack's flag, as drawn and compiled"
    );
}

#[test]
fn a_pack_without_cldr_data_for_its_language_is_refused() {
    let root = Root::new();
    root.write_bytes("cs", "cldr.postcard", &base_blob());
    let (translations, report) = Translations::load(&[root.path()]);
    assert_eq!(report.refused.len(), 1, "{report}");
    assert_eq!(report.refused[0].0.tag(), "cs");
    assert_eq!(translations.languages(), [Language::EN]);
}

#[test]
fn a_pack_with_no_cldr_blob_is_refused_on_its_facts_even_where_the_base_blob_speaks_it() {
    let root = Root::new();
    root.write("cs", "info.json", r#"{ "name": "Čeština" }"#)
        .write(
            "en-GB",
            "system.json",
            &translator_file(&[("colour", "Color", Some("Colour"))]),
        );
    let (translations, report) = Translations::read(&[root.path()]);
    assert_eq!(translations.languages(), [Language::EN], "{report}");
    assert_eq!(
        report
            .refused
            .iter()
            .map(|(language, _)| language.tag())
            .collect::<Vec<_>>(),
        ["cs", "en-GB"],
        "{report}"
    );
}

#[test]
fn a_pack_is_named_by_its_pack_language_which_is_neither_english_nor_und() {
    let root = Root::new();
    root.language("zh-Hans")
        .language("sr-Cyrl")
        .language("CS")
        .language("en")
        .language("und")
        .language("und-Latn");
    let (_, report) = Translations::load(&[root.path()]);
    let skipped = skipped(&report);
    assert!(skipped.contains("zh-Hans: name it `zh`"), "{report}");
    assert!(skipped.contains("sr-Cyrl: name it `sr`"), "{report}");
    assert!(skipped.contains("CS: name it `cs`"), "{report}");
    assert!(skipped.contains("English is the code's own"), "{report}");
    assert!(skipped.contains("und: names no language"), "{report}");
    assert!(skipped.contains("und-Latn: names no language"), "{report}");
    assert!(report.languages.is_empty(), "{report}");
}

#[test]
fn a_pack_the_fonts_cannot_draw_or_written_right_to_left_is_refused() {
    let root = Root::new();
    root.language("ar");
    root.write(
        "cs",
        "info.json",
        r#"{ "name": "Čeština", "direction": "rtl" }"#,
    );
    let (_, report) = Translations::load(&[root.path()]);
    let refused: Vec<String> = report
        .refused
        .iter()
        .map(|(language, reason)| format!("{language}: {reason}"))
        .collect();
    assert_eq!(
        refused,
        [
            "ar: written in Arab, which the device's fonts do not draw",
            "cs: written right to left, which the device does not lay out yet",
        ]
    );
}

#[test]
fn a_language_refused_in_every_root_is_reported_once() {
    let (earlier, later) = (Root::new(), Root::new());
    earlier.language("ar");
    later.language("ar");
    let (_, report) = Translations::load(&[earlier.path(), later.path()]);
    assert_eq!(report.refused.len(), 1, "{report}");
}

#[test]
fn a_widgets_directory_that_cannot_be_listed_is_reported() {
    let root = Root::new();
    root.write("zh", "widgets", "a file, not a directory");
    let (_, report) = Translations::load(&[profile(), root.path()]);
    assert!(
        skipped(&report).contains("zh/widgets: unreadable"),
        "{report}"
    );
}

#[test]
fn a_later_roots_misspelt_direction_is_skipped_and_the_language_stays() {
    let later = Root::new();
    later.write(
        "zh",
        "info.json",
        r#"{ "name": "简体中文", "direction": "RTL" }"#,
    );
    let (translations, report) = Translations::load(&[profile(), later.path()]);
    assert!(
        skipped(&report).contains("info.json: not a pack's info: unknown variant `RTL`"),
        "{report}"
    );
    assert_eq!(
        translations.languages(),
        [Language::EN, Language::ZH],
        "{report}"
    );
}

#[test]
fn a_later_root_turning_a_language_right_to_left_withdraws_it() {
    let later = Root::new();
    later
        .write(
            "zh",
            "info.json",
            r#"{ "name": "简体中文", "direction": "rtl" }"#,
        )
        .write(
            "zh",
            "web.json",
            &translator_file(&[("web", "Settings", Some("设置"))]),
        );
    let (translations, report) = Translations::load(&[profile(), later.path()]);
    assert_eq!(translations.languages(), [Language::EN], "{report}");
    assert_eq!(report.refused.len(), 1, "{report}");
    assert!(report.languages.is_empty(), "{report}");
    assert!(translations.web(Language::ZH).is_none(), "{report}");
}

#[test]
fn a_later_root_turning_a_language_left_to_right_offers_it_unrefused() {
    let earlier = Root::new();
    earlier.write(
        "zh",
        "info.json",
        r#"{ "name": "简体中文", "direction": "rtl" }"#,
    );
    let (translations, report) = Translations::load(&[earlier.path(), profile()]);
    assert_eq!(
        translations.languages(),
        [Language::EN, Language::ZH],
        "{report}"
    );
    assert!(report.is_clean(), "{report}");
}

#[test]
fn an_entry_not_shaped_as_intl_writes_it_is_named_not_taken_as_untranslated() {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        r#"{
            "bare": "探针",
            "keyless": { "source": "Probe" },
            "good": { "source": "Clear", "translation": "晴" }
        }"#,
    );
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    let skipped = skipped(&report);
    assert!(
        skipped.contains("`bare` is not an entry object"),
        "{report}"
    );
    assert!(
        skipped.contains("`keyless` has no `translation`"),
        "{report}"
    );
    let zh = translations.formatter(Language::ZH);
    let good = format_message!(zh, id: "good", default_message: "Clear");
    assert_eq!(good, "晴", "{report}");
}

#[test]
fn a_broken_entry_is_left_out_and_named_and_the_rest_loads() {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[
            ("good", "Clear", Some("晴")),
            ("broken", "Connect to {ssid}", Some("连接到 {ssid")),
            ("renamed", "Connect to {ssid}", Some("连接到 {network}")),
            ("dropped", "Connect to {ssid}", Some("连接")),
            ("waiting", "Loading", None),
        ]),
    );
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    assert_eq!(report.invalid.len(), 3, "{report}");
    let problem = |id: &str| {
        report
            .invalid
            .iter()
            .find(|line| line.contains(&format!("`{id}`")))
            .unwrap_or_else(|| panic!("`{id}` is not named in {report}"))
    };
    problem("broken");
    let renamed = problem("renamed");
    assert!(
        renamed.contains("[{ssid}]") && renamed.contains("[{network}]"),
        "{report}"
    );
    assert!(
        problem("dropped").contains("arguments are [{ssid}], the translation's []"),
        "{report}"
    );
    let zh = translations.formatter(Language::ZH);
    let good = format_message!(zh, id: "good", default_message: "Clear");
    assert_eq!(good, "晴");
    let waiting = format_message!(zh, id: "waiting", default_message: "Loading");
    assert_eq!(
        waiting, "Loading",
        "an untranslated entry speaks its English"
    );
}

#[test]
fn a_compiled_flag_is_taken_only_whole() {
    let compiled = profile_zh("flag.bin");
    assert_eq!(
        bmc_ui::svg::compiled_svg_len(&compiled),
        Some(compiled.len()),
        "the profile pack's flag is one whole icon"
    );

    for (name, contents, problem) in [
        (
            "pointer",
            LFS_POINTER_FILE.as_bytes().to_vec(),
            "a git-lfs pointer, not the file",
        ),
        (
            "truncated",
            compiled[..compiled.len() - 4].to_vec(),
            "cut short, run on past its end",
        ),
    ] {
        let root = Root::new();
        root.write_bytes("zh", "flag.bin", &contents);
        let (translations, report) = Translations::load(&[profile(), root.path()]);
        assert!(skipped(&report).contains(problem), "{name}: {report}");
        assert_eq!(
            translations.flag_bin(Language::ZH),
            Some(profile().join("zh/flag.bin").as_path()),
            "{name}: the profile's flag stays"
        );
    }
}

#[test]
fn a_file_that_cannot_be_read_is_reported_and_a_missing_one_is_not() {
    let root = Root::new();
    root.write_bytes("zh", "system.json", b"{\"probe\": \"\xff\"}");
    let (_, report) = Translations::load(&[profile(), root.path()]);
    let skipped = skipped(&report);
    assert!(
        skipped.contains("system.json: unreadable"),
        "not UTF-8: {report}"
    );
    assert!(
        !skipped.contains(units::FILE),
        "the later root brings no units.postcard, which is no fault: {report}"
    );
}

#[test]
fn a_file_without_read_permission_is_reported() {
    use std::os::unix::fs::PermissionsExt;

    let root = Root::new();
    root.write("zh", "info.json", r#"{ "name": "简体中文" }"#);
    let info = root.path().join("zh/info.json");
    std::fs::set_permissions(&info, std::fs::Permissions::from_mode(0o000))
        .expect("BUG: a writable temporary directory");
    if std::fs::read(&info).is_ok() {
        return; // Running as root, which reads regardless.
    }
    let (_, report) = Translations::load(&[profile(), root.path()]);
    assert!(
        skipped(&report).contains("info.json: unreadable"),
        "{report}"
    );
}

#[test]
fn what_a_pack_brings_beside_its_translations_is_skipped_when_it_does_not_parse() {
    let root = Root::new();
    root.write("zh", "info.json", r#"{ "nom": "x" }"#)
        .write("zh", units::FILE, "not units")
        .write("zh", "flag.bin", "<svg")
        .write("zh", "web.json", "[]");
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    let skipped = skipped(&report);
    assert!(skipped.contains("not a pack's info"), "{report}");
    assert!(
        skipped.contains("units.postcard: not a language's units"),
        "{report}"
    );
    assert!(
        skipped.contains("not a compiled flag: no drawable viewBox"),
        "{report}"
    );
    assert!(skipped.contains("web.json: not a JSON object"), "{report}");
    assert_eq!(
        report.languages,
        [Language::ZH],
        "a bad file is skipped, the language stays"
    );
    assert_eq!(
        translations.flag_bin(Language::ZH),
        Some(profile().join("zh/flag.bin").as_path()),
        "the profile's compiled flag stays where the later root's does not open"
    );
    assert!(
        translations.units(Language::ZH).is_some(),
        "the profile's units stay where the later root's do not parse"
    );
}

#[test]
fn web_and_widget_translations_are_kept_by_language() {
    let root = Root::new();
    root.write(
        "zh",
        "web.json",
        &translator_file(&[("web", "Settings", Some("设置"))]),
    )
    .write(
        "zh",
        "widgets/clock.json",
        &translator_file(&[("tick", "Tick", Some("滴答"))]),
    );
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    assert!(report.is_clean(), "{report}");
    let web = translations
        .web(Language::ZH)
        .expect("BUG: the pack's web.json");
    assert_eq!(web.get("web").map(String::as_str), Some("设置"));
    let clock = translations
        .widget("clock", Language::ZH)
        .expect("BUG: the pack's widget file");
    assert_eq!(clock.get("tick").map(String::as_str), Some("滴答"));
}

#[test]
fn every_simplified_chinese_spelling_reaches_the_zh_pack() -> R {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[("probe", "Probe", Some("探针"))]),
    )
    .write(
        "zh",
        "web.json",
        &translator_file(&[("web", "Settings", Some("设置"))]),
    );
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    assert!(report.is_clean(), "{report}");
    for setting in ["zh-Hans", "zh-CN", "zh-Hans-CN"] {
        let language = Language::parse(setting)?;
        let intl = translations.formatter(language);
        assert_eq!(intl.locale().to_string(), "zh", "{setting}");
        let probe = format_message!(intl, id: "probe", default_message: "Probe");
        assert_eq!(probe, "探针", "{setting}");
        assert_eq!(
            translations.info(language).native_name,
            "简体中文",
            "{setting}"
        );
        assert!(translations.units(language).is_some(), "{setting}");
        assert!(translations.web(language).is_some(), "{setting}");
    }
    assert!(
        translations.web(Language::parse("zh-Hant")?).is_none(),
        "Traditional Chinese does not fall back to Simplified"
    );
    Ok(())
}

#[test]
fn each_load_is_a_later_version() {
    let (first, _) = Translations::load(&[]);
    let (second, _) = Translations::load(&[]);
    assert!(second.version() > first.version());
    let (read, _) = Translations::read(&[]);
    assert!(read.version() > second.version(), "reading the facts");
}

#[test]
fn a_reload_puts_a_later_version_in_effect() {
    let _process_wide = PROCESS_WIDE.lock().unwrap_or_else(PoisonError::into_inner);
    let before = Translations::shared().version();
    let report = Translations::reload();
    assert!(Translations::shared().version() > before, "{report}");
}

/// A `cs` pack whose name, `info.json` and flags are sound, and whose every other file is not.
fn pack_with_broken_contents(root: &Root) {
    let (flag_svg, flag_bin) = (profile_zh("flag.svg"), profile_zh("flag.bin"));
    root.write("cs", "info.json", r#"{ "name": "Čeština" }"#)
        .write_bytes("cs", "flag.svg", &flag_svg)
        .write_bytes("cs", "flag.bin", &flag_bin)
        .write("cs", "system.json", "not json")
        .write("cs", "web.json", "[")
        .write("cs", "widgets/clock.json", "x")
        .write("cs", units::FILE, "x")
        .write("cs", "cldr.postcard", "not a blob");
}

#[test]
fn reading_the_facts_opens_no_translation_or_cldr_data() -> R {
    let root = Root::new();
    pack_with_broken_contents(&root);
    let cs = Language::parse("cs")?;

    let (translations, report) = Translations::read(&[root.path()]);
    assert!(report.is_clean(), "{report}");
    assert_eq!(translations.languages(), [Language::EN, cs]);
    assert_eq!(translations.info(cs).native_name, "Čeština");
    assert!(
        translations.flag_svg(cs).is_some() && translations.flag_bin(cs).is_some(),
        "the pack's flags"
    );
    assert_eq!(translations.loads(cs), 0, "the facts load no pack");

    let (_, report) = translations.load_all();
    let skipped = skipped(&report);
    for file in [
        "system.json",
        "web.json",
        "widgets/clock.json",
        units::FILE,
        "cldr.postcard",
    ] {
        assert!(skipped.contains(file), "{file}: {report}");
    }
    assert_eq!(report.refused.len(), 1, "{report}");
    Ok(())
}

#[test]
fn reading_the_facts_passes_over_contents_that_cannot_be_opened() -> R {
    use std::os::unix::fs::PermissionsExt;

    let root = Root::new();
    root.write("cs", "info.json", r#"{ "name": "Čeština" }"#)
        .write("cs", "system.json", "{}")
        .write("cs", "cldr.postcard", "");
    for file in ["system.json", "cldr.postcard"] {
        std::fs::set_permissions(
            root.path().join("cs").join(file),
            std::fs::Permissions::from_mode(0o000),
        )?;
    }
    if std::fs::read(root.path().join("cs/system.json")).is_ok() {
        return Ok(()); // Running as root, which reads regardless.
    }
    let (translations, report) = Translations::read(&[root.path()]);
    assert!(report.is_clean(), "{report}");
    assert_eq!(
        translations.languages(),
        [Language::EN, Language::parse("cs")?]
    );
    Ok(())
}

#[test]
fn a_pack_that_fails_to_load_speaks_english_once_asked() -> R {
    let root = Root::new();
    pack_with_broken_contents(&root);
    let cs = Language::parse("cs")?;
    let (translations, _) = Translations::read(&[root.path()]);
    for _ in 0..2 {
        let intl = translations.formatter(cs);
        let probe = format_message!(intl, id: "probe", default_message: "Probe");
        assert_eq!(probe, "Probe");
    }
    assert_eq!(translations.loads(cs), 1, "a failed load is not retried");
    assert!(translations.units(cs).is_none() && translations.web(cs).is_none());
    Ok(())
}

#[test]
fn a_pack_that_fails_to_load_gives_way_to_the_next_along_the_fallback_chain() -> R {
    let root = Root::new();
    root.write_bytes("zh-SG", "cldr.postcard", &base_blob())
        .write("zh-SG", "info.json", r#"{ "name": "新加坡华语" }"#)
        .write(
            "zh",
            "system.json",
            &translator_file(&[("probe", "Probe", Some("探针"))]),
        );
    let (translations, _) = Translations::read(&[profile(), root.path()]);
    let zh_sg = Language::parse("zh-SG")?;
    assert!(
        translations.languages().contains(&zh_sg),
        "offered on its facts"
    );
    let intl = translations.formatter(zh_sg);
    let probe = format_message!(intl, id: "probe", default_message: "Probe");
    assert_eq!(probe, "探针");
    assert!(
        !translations.languages().contains(&zh_sg),
        "no longer offered once its load is refused"
    );
    assert_eq!(
        translations.info(zh_sg).native_name,
        translations.info(Language::ZH).native_name,
        "its name, as its text, comes from the pack it gave way to"
    );
    Ok(())
}

#[test]
fn a_pack_loads_once_however_many_threads_ask() {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[("probe", "Probe", Some("探针"))]),
    );
    let (translations, _) = Translations::read(&[profile(), root.path()]);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let zh = translations.formatter(Language::ZH);
                let probe = format_message!(zh, id: "probe", default_message: "Probe");
                assert_eq!(probe, "探针");
            });
        }
    });
    assert_eq!(translations.loads(Language::ZH), 1);
}

#[test]
fn a_pack_stands_on_its_own_cldr_data() {
    let root = Root::new();
    root.language("zh-SG");
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    let refused: Vec<String> = report
        .refused
        .iter()
        .map(|(language, _)| language.tag())
        .collect();
    assert_eq!(
        refused,
        ["zh-SG"],
        "the zh pack's data does not stand in for a regional pack's own: {report}"
    );
    assert_eq!(translations.languages(), [Language::EN, Language::ZH]);
}

/// A `zh` pack translating `a`, `b` and a `web.json` entry,
/// and a `zh-SG` one over it with its own blob, translating `a`.
fn regional_over_parent() -> Root {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[("a", "A", Some("甲")), ("b", "B", Some("乙"))]),
    )
    .write(
        "zh",
        "web.json",
        &translator_file(&[("web", "Web", Some("网"))]),
    )
    .write_bytes("zh-SG", "cldr.postcard", &profile_zh("cldr.postcard"))
    .write(
        "zh-SG",
        "system.json",
        &translator_file(&[("a", "A", Some("甲（新）"))]),
    );
    root
}

#[test]
fn a_regional_pack_layers_over_its_parent_per_entry() -> R {
    let root = regional_over_parent();
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    let zh_sg = Language::parse("zh-SG")?;
    assert!(translations.languages().contains(&zh_sg), "{report}");
    let intl = translations.formatter(zh_sg);
    let own = format_message!(intl, id: "a", default_message: "A");
    let inherited = format_message!(intl, id: "b", default_message: "B");
    assert_eq!(own, "甲（新）", "its own entry stands over its parent's");
    assert_eq!(inherited, "乙", "an entry it lacks is its parent's");
    assert_eq!(
        translations
            .web(zh_sg)
            .and_then(|web| web.get("web").cloned())
            .as_deref(),
        Some("网"),
        "its parent's web UI"
    );
    assert_eq!(
        translations.info(zh_sg).native_name,
        "zh-SG",
        "a name is a pack's own, never its parent's"
    );
    assert!(
        translations.flag_svg(Language::ZH).is_some() && translations.flag_svg(zh_sg).is_none(),
        "a flag names a region, so none comes from the parent"
    );
    Ok(())
}

#[test]
fn the_nearest_parent_translating_an_entry_wins() -> R {
    let root = Root::new();
    root.write(
        "es",
        "system.json",
        &translator_file(&[("both", "Both", Some("es")), ("far", "Far", Some("es"))]),
    )
    .write(
        "es-419",
        "system.json",
        &translator_file(&[("both", "Both", Some("es-419")), ("far", "Far", None)]),
    )
    .write_bytes("es-MX", "cldr.postcard", &es_mx_blob());
    let (translations, report) = Translations::load(&[root.path()]);
    let es_mx = Language::parse("es-MX")?;
    assert!(
        translations.languages().contains(&es_mx),
        "if the fixture was cut by another ICU4X, re-cut it with \
         `nix run .#intl-cldr-blob -- es-MX bmc-intl/core/{ES_MX_FIXTURE}`: {report}"
    );
    let intl = translations.formatter(es_mx);
    let both = format_message!(intl, id: "both", default_message: "Both");
    let far = format_message!(intl, id: "far", default_message: "Far");
    assert_eq!(both, "es-419", "`es-MX` layers over `es-419`, then `es`");
    assert_eq!(far, "es", "a parent's null shows the next one's text");
    Ok(())
}

#[test]
fn a_refused_regional_pack_names_only_its_own_files_as_left_out() {
    let root = regional_over_parent();
    root.write_bytes("zh-SG", "cldr.postcard", &base_blob())
        .write(
            "zh-SG",
            "widgets/clock.json",
            &translator_file(&[("clock", "Clock", Some("钟"))]),
        );
    let (_, report) = Translations::load(&[profile(), root.path()]);
    assert!(
        report
            .refused
            .iter()
            .any(|(language, _)| language.tag() == "zh-SG"),
        "{report}"
    );
    let skipped = skipped(&report);
    assert!(
        skipped.contains("widgets/clock.json in zh-SG"),
        "its own file is named: {report}"
    );
    assert!(
        !skipped.contains("web.json in zh-SG"),
        "the web.json is its parent's: {report}"
    );
}

#[test]
fn a_parent_refused_on_its_facts_still_lends_its_text() -> R {
    let root = regional_over_parent();
    let (translations, report) = Translations::load(&[root.path()]);
    let zh_sg = Language::parse("zh-SG")?;
    assert!(
        !translations.languages().contains(&Language::ZH),
        "`zh` brings no blob of its own: {report}"
    );
    assert!(translations.languages().contains(&zh_sg), "{report}");
    let intl = translations.formatter(zh_sg);
    let inherited = format_message!(intl, id: "b", default_message: "B");
    assert_eq!(inherited, "乙", "{report}");
    Ok(())
}

#[test]
fn a_parents_finding_is_reported_once_with_a_regional_pack_over_it() {
    let root = regional_over_parent();
    root.write("zh", "widgets/clock.json", "x");
    let (_, report) = Translations::load(&[profile(), root.path()]);
    let skipped = skipped(&report);
    assert_eq!(skipped.matches("clock.json").count(), 1, "{report}");
}

#[test]
fn a_pack_refused_on_its_facts_is_never_loaded() {
    let root = Root::new();
    root.write(
        "cs",
        "info.json",
        r#"{ "name": "Čeština", "direction": "rtl" }"#,
    )
    .write("cs", "system.json", "not json");
    let (_, report) = Translations::load(&[root.path()]);
    assert_eq!(report.refused.len(), 1, "{report}");
    assert!(!skipped(&report).contains("system.json"), "{report}");
}

#[test]
fn the_base_blob_answers_every_argument_kind_the_runtime_formats() {
    let (translations, _) = Translations::load(&[]);
    let intl = translations.formatter(Language::EN);
    let spoken = format_message!(
        intl,
        id: "kinds",
        default_message: "{count, plural, one {{count} probe} other {{count} probes}}, {rank, selectordinal, one {{rank}st} other {{rank}th}}, {state, select, on {on} other {off}}",
        values: { count: 2_i64, rank: 1_i64, state: "on" },
    );
    assert_eq!(spoken, "2 probes, 1st, on");
}

#[test]
fn a_lazy_message_is_worded_by_id_and_falls_back_to_its_english() {
    let root = Root::new();
    root.write(
        "zh",
        "system.json",
        &translator_file(&[("probe", "Probe", Some("探针"))]),
    );
    let (translations, _) = Translations::load(&[profile(), root.path()]);
    let translated = Message {
        id: "probe",
        default_message: "Probe",
        description: Some("A test fixture"),
    };
    let untranslated = Message {
        id: "probe.untranslated",
        default_message: "Untranslated probe",
        description: None,
    };

    let zh = translations.formatter(Language::ZH);
    assert_eq!(format_lazy(&zh, &translated), "探针");
    assert_eq!(format_lazy(&zh, &untranslated), "Untranslated probe");

    let en = translations.formatter(Language::EN);
    assert_eq!(format_lazy(&en, &translated), "Probe");
}

#[test]
fn a_pattern_parses_as_the_runtime_parses_it() {
    assert!(parse_pattern("Connect to {ssid}").is_ok());
    assert!(parse_pattern("Connect to {ssid").is_err());
    assert!(
        parse_pattern("{count, plural, one {{count} probe}}").is_err(),
        "the runtime requires an `other` branch"
    );
}

#[test]
fn a_later_roots_units_stand_over_earlier_ones_and_a_broken_one_stands_over_none() {
    let profiles = profile_zh(units::FILE);
    let profiles = units::Units::from_bytes(&profiles).expect("BUG: the profile's units");
    let marked = units::Units {
        range: "{0}~{1}".into(),
        ..profiles
    };
    let broken = Root::new();
    broken.write("zh", units::FILE, "not units");
    let later = Root::new();
    later.write_bytes("zh", units::FILE, &marked.to_bytes());

    let (translations, report) = Translations::load(&[profile(), later.path(), broken.path()]);
    assert!(
        skipped(&report).contains("not a language's units"),
        "the broken one is reported: {report}"
    );
    let units = translations
        .units(Language::ZH)
        .expect("BUG: the later root's units load");
    assert_eq!(
        units.get().range,
        "{0}~{1}",
        "the later root's units stand over the profile's, the broken last root takes nothing"
    );
}

#[test]
fn a_later_roots_blob_that_does_not_open_leaves_the_earlier_one_standing() {
    let root = Root::new();
    root.write("zh", "cldr.postcard", LFS_POINTER_FILE);
    let (translations, report) = Translations::load(&[profile(), root.path()]);
    assert!(
        skipped(&report).contains("cldr.postcard: not an ICU4X blob: a git-lfs pointer"),
        "{report}"
    );
    assert!(
        report.refused.is_empty() && translations.languages().contains(&Language::ZH),
        "Chinese still formats with the profile's blob: {report}"
    );
}
