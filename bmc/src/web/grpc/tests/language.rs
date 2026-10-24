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

//! The languages over gRPC: the list reachable before anyone logs in, the setting behind a session,
//! and both absent without the i18n marker.

use std::sync::Arc;

use bmc_grpc::web::{ConfigurationServiceClient, LanguageServiceClient, SetLanguageRequest};
use bmc_platform::{HardwareProfile, Product};
use buffa_types::google::protobuf::Empty;
use connectrpc::{ErrorCode, Router};

use super::super::{gated, language::LanguageService, needs_i18n};
use super::boser_managed::{I18N_MARKER, production_router, with_session};
use super::{grpc, grpc_web, transport};
use crate::i18n::I18nMarker;
use crate::test_support::StubSessionManager;

/// A gated router serving the language list, with the marker in `dir` when `set`.
fn language_router(dir: &tempfile::TempDir, set: bool) -> axum::Router {
    let marker = dir.path().join(I18N_MARKER);
    if set {
        std::fs::write(&marker, "").expect("BUG: a writable temporary directory");
    }
    gated(
        Router::new().add_service(Arc::new(LanguageService)),
        Arc::new(StubSessionManager),
        HardwareProfile::for_product(Product::Bmc100).capabilities(),
        I18nMarker::new(marker),
    )
}

#[tokio::test]
async fn with_the_marker_the_languages_are_listed_without_a_session() {
    let dir = tempfile::tempdir().expect("BUG: a temporary directory");
    let router = language_router(&dir, true);

    for (protocol, config) in [("gRPC", grpc()), ("gRPC-Web", grpc_web())] {
        let listed = LanguageServiceClient::new(transport(router.clone()), config)
            .list_languages(Empty::default())
            .await
            .unwrap_or_else(|error| panic!("BUG: {protocol}: the gate let it through: {error:?}"))
            .into_owned();
        assert_eq!(
            listed.languages.first().map(|info| info.tag.as_str()),
            Some("en-GB"),
            "{protocol}"
        );
    }
}

#[tokio::test]
async fn without_the_marker_the_device_answers_as_a_bmc_application_without_languages() {
    let dir = tempfile::tempdir().expect("BUG: a temporary directory");
    let router = language_router(&dir, false);
    let unserved = gated(
        Router::new(),
        Arc::new(StubSessionManager),
        HardwareProfile::for_product(Product::Bmc100).capabilities(),
        I18nMarker::unset(),
    );

    for (protocol, config) in [("gRPC", grpc()), ("gRPC-Web", grpc_web())] {
        let gated = LanguageServiceClient::new(transport(router.clone()), config.clone())
            .list_languages(Empty::default())
            .await
            .expect_err("no marker, no language list");
        let older = LanguageServiceClient::new(transport(unserved.clone()), config)
            .list_languages(Empty::default())
            .await
            .expect_err("nothing serves the list");
        assert_eq!(
            (gated.code, gated.message),
            (older.code, older.message),
            "{protocol}"
        );
    }
}

#[tokio::test]
async fn with_the_marker_setting_the_language_needs_a_session() {
    let (dir, router) = production_router(Product::Bmc100).await;
    std::fs::write(dir.path().join(I18N_MARKER), "").expect("BUG: a writable tempdir");

    let error = ConfigurationServiceClient::new(transport(router), grpc())
        .set_language(set_language("zh"))
        .await
        .expect_err("no session, no setting");
    assert_eq!(error.code, ErrorCode::Unauthenticated);
}

#[tokio::test]
async fn with_the_marker_a_set_language_is_the_one_the_settings_report() {
    let (dir, router) = production_router(Product::Bmc100).await;
    std::fs::write(dir.path().join(I18N_MARKER), "").expect("BUG: a writable tempdir");
    let client = ConfigurationServiceClient::new(transport(with_session(router)), grpc());

    client
        .set_language(set_language("en-US"))
        .await
        .expect("BUG: an offered language is taken");
    let settings = client
        .get_general_settings_data(Empty::default())
        .await
        .expect("BUG: the settings are read")
        .into_owned();
    assert_eq!(settings.language.as_deref(), Some("en-US"));
}

#[tokio::test]
async fn setting_the_language_in_effect_saves_nothing() {
    use std::os::unix::fs::MetadataExt as _;

    let (dir, router) = production_router(Product::Bmc100).await;
    std::fs::write(dir.path().join(I18N_MARKER), "").expect("BUG: a writable tempdir");
    let client = ConfigurationServiceClient::new(transport(with_session(router)), grpc());
    let config = dir.path().join("config.json");
    let inode = || {
        std::fs::metadata(&config)
            .expect("BUG: the first setting saved the config")
            .ino()
    };

    client
        .set_language(set_language("en-US"))
        .await
        .expect("BUG: an offered language is taken");
    let saved = inode();
    client
        .set_language(set_language("en-US"))
        .await
        .expect("BUG: the language in effect is taken");

    assert_eq!(inode(), saved, "a save replaces the file; none happened");
}

#[tokio::test]
async fn without_the_marker_the_settings_name_no_language_and_none_is_set() {
    let (_dir, router) = production_router(Product::Bmc100).await;
    let client = ConfigurationServiceClient::new(transport(with_session(router)), grpc());

    let settings = client
        .get_general_settings_data(Empty::default())
        .await
        .expect("BUG: the settings are read")
        .into_owned();
    assert_eq!(settings.language, None);
    let error = client
        .set_language(set_language("en-US"))
        .await
        .expect_err("no marker, no setting");
    assert_eq!(error.code, ErrorCode::Unimplemented);
}

fn set_language(tag: &str) -> SetLanguageRequest {
    SetLanguageRequest {
        language: tag.to_owned(),
    }
}

#[test]
fn the_marker_gates_the_language_list_and_setting_alone() {
    let gated_procedures = [
        "/braiins.bmc.web.LanguageService/ListLanguages",
        "/braiins.bmc.web.LanguageService/GetWebCatalog",
        "/braiins.bmc.web.ConfigurationService/SetLanguage",
    ];
    for path in gated_procedures {
        assert!(needs_i18n(path), "{path}");
    }
    for path in [
        "/braiins.bmc.web.ConfigurationService/SetDateFormat",
        "/braiins.bmc.web.ConfigurationService/GetGeneralSettingsData",
    ] {
        assert!(!needs_i18n(path), "{path}");
    }
}
