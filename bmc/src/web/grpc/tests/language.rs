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

//! The language list is reachable before anyone logs in.

use std::sync::Arc;

use bmc_grpc::web::LanguageServiceClient;
use bmc_platform::{HardwareProfile, Product};
use buffa_types::google::protobuf::Empty;
use connectrpc::Router;

use super::super::{gated, language::LanguageService};
use super::{grpc, grpc_web, transport};
use crate::test_support::StubSessionManager;

#[tokio::test]
async fn the_languages_are_listed_without_a_session() {
    let router = gated(
        Router::new().add_service(Arc::new(LanguageService)),
        Arc::new(StubSessionManager),
        HardwareProfile::for_product(Product::Bmc100).capabilities(),
    );

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
