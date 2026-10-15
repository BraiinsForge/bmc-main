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

//! The metadata RPCs through the production routes.

use super::boser_managed::{upgrade_router, with_session};
use super::{grpc, transport};
use bmc_grpc::web::{GetMetadataRequest, MetadataServiceClient};
use bmc_platform::Product;

#[tokio::test]
async fn get_metadata_carries_the_packages_version() {
    let (_tempdir, router) = upgrade_router(Product::Bmc100, None, None).await;
    let metadata = MetadataServiceClient::new(transport(with_session(router)), grpc())
        .get_metadata(GetMetadataRequest::default())
        .await
        .expect("BUG: GetMetadata succeeds")
        .into_owned();

    assert_eq!(metadata.version, "current");
    assert_eq!(
        metadata.packages_version.as_deref(),
        Some("2026.09.18-39e9db98-26.10")
    );
}
