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

use base64::Engine as _;
use bmc_grpc::web::{
    self, Icon, LanguageInfo, LanguageService as GrpcLanguageService, ListLanguagesResponse,
};
use bmc_intl::{Language, TextDirection, Translations};
use buffa_types::google::protobuf::Empty;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use tracing::warn;

use super::shared::invalid_field;

pub(crate) struct LanguageService;

impl GrpcLanguageService for LanguageService {
    async fn list_languages(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListLanguagesResponse> {
        Response::ok(list(&Translations::shared()))
    }
}

fn list(translations: &Translations) -> ListLanguagesResponse {
    let languages = translations
        .languages()
        .into_iter()
        .map(|language| {
            let info = translations.info(language);
            LanguageInfo {
                tag: language.tag(),
                native_name: info.native_name,
                text_direction: match info.text_direction {
                    TextDirection::Ltr => web::TextDirection::TEXT_DIRECTION_LTR,
                    TextDirection::Rtl => web::TextDirection::TEXT_DIRECTION_RTL,
                }
                .into(),
                flag: flag(translations, language).into(),
            }
        })
        .collect();
    ListLanguagesResponse { languages }
}

/// The language `tag` names, when it is one [`list`] offers, spelled as it lists it:
/// a picker sends back what it was given, so any other spelling is a client's mistake.
pub(crate) fn offered_language(
    translations: &Translations,
    tag: &str,
) -> Result<Language, ConnectError> {
    Language::parse(tag)
        .ok()
        .filter(|language| language.tag() == tag && translations.languages().contains(language))
        .ok_or_else(|| invalid_field("language", "not a language this device offers"))
}

/// Left out, with a warning, when the pack's flag cannot be read.
fn flag(translations: &Translations, language: Language) -> Option<Icon> {
    match translations.flag_svg(language) {
        Ok(svg) => svg.map(|svg| Icon {
            mime_type: "image/svg+xml".to_owned(),
            data: base64::engine::general_purpose::STANDARD.encode(svg),
        }),
        Err(error) => {
            warn!(%error, %language, "language flag unreadable; left out");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use base64::Engine as _;
    use bmc_grpc::web;
    use bmc_intl::Translations;
    use connectrpc::ErrorCode;

    use super::{list, offered_language};

    fn profile() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/intl/profile-languages")
    }

    #[test]
    fn the_englishes_then_the_profile_packs_are_listed_each_named_in_itself() {
        let (translations, _) = Translations::load(&[profile()]);
        let listed: Vec<(String, String)> = list(&translations)
            .languages
            .into_iter()
            .map(|info| (info.tag, info.native_name))
            .collect();
        let named = |tag: &str, name: &str| (tag.to_owned(), name.to_owned());
        assert_eq!(
            listed,
            [
                named("en-GB", "English (UK)"),
                named("en-US", "English (US)"),
                named("zh", "简体中文"),
            ]
        );
    }

    #[test]
    fn every_listed_flag_is_an_svg_and_a_pack_without_one_is_named_alone() {
        let root = tempfile::tempdir().expect("BUG: a temporary directory");
        std::fs::remove_file(copy_profile_zh(root.path()).join("flag.svg"))
            .expect("BUG: the copied pack has a flag");
        let (translations, report) = Translations::load(&[root.path().to_owned()]);
        assert!(report.is_clean(), "{report}");

        for info in list(&translations).languages {
            assert_eq!(info.text_direction, web::TextDirection::TEXT_DIRECTION_LTR);
            match info.flag.as_option() {
                None => assert_eq!(info.tag, "zh", "only the flagless pack goes without"),
                Some(icon) => {
                    assert_eq!(icon.mime_type, "image/svg+xml");
                    let svg = base64::engine::general_purpose::STANDARD
                        .decode(&icon.data)
                        .expect("BUG: the flag is base64");
                    assert!(svg.starts_with(b"<svg"), "{}", info.tag);
                }
            }
        }
    }

    #[test]
    fn only_a_listed_tag_spelled_as_listed_sets_the_language() {
        let (translations, _) = Translations::load(&[profile()]);
        for tag in ["en-GB", "en-US", "zh"] {
            let language = offered_language(&translations, tag)
                .unwrap_or_else(|error| panic!("BUG: {tag} is listed: {error:?}"));
            assert_eq!(language.tag(), tag);
        }
        for tag in ["en", "en-gb", "zh-Hans", "de", "not a tag", ""] {
            let error = offered_language(&translations, tag)
                .expect_err("only a listed spelling of a listed language is taken");
            assert_eq!(error.code, ErrorCode::InvalidArgument, "{tag:?}");
        }
    }

    /// The profile's `zh` pack copied under `root`, for a test to take a file away.
    fn copy_profile_zh(root: &std::path::Path) -> PathBuf {
        let pack = root.join("zh");
        std::fs::create_dir(&pack).expect("BUG: a writable temporary directory");
        for entry in std::fs::read_dir(profile().join("zh")).expect("BUG: the profile pack") {
            let path = entry.expect("BUG: a readable entry").path();
            std::fs::copy(
                &path,
                pack.join(path.file_name().expect("BUG: a file name")),
            )
            .expect("BUG: a writable temporary directory");
        }
        pack
    }
}
