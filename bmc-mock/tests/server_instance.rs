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

//! Server instance ID tests over gRPC. The ID is served unauthenticated,
//! but each channel still logs in with the per-instance password first to
//! pin the connection to the intended mock (see common::authenticated_channel).

mod common;

use bmc_grpc::web::GetServerInstanceRequest;
use bmc_grpc::web::metadata_service_client::MetadataServiceClient;
use tonic::transport::Channel;
use uuid::Uuid;

const SCENARIO: &str = r#"{"firmware": "up-to-date", "packages": "unavailable"}"#;

async fn fetch_instance_id(channel: Channel) -> Uuid {
    let id = MetadataServiceClient::new(channel)
        .get_server_instance(GetServerInstanceRequest {})
        .await
        .expect("BUG: get_server_instance failed")
        .into_inner()
        .server_instance_id;
    let parsed = Uuid::parse_str(&id).expect("server instance id is a valid UUID");
    assert_eq!(parsed.get_version_num(), 4, "server instance id is v4");
    parsed
}

#[tokio::test]
async fn same_process_reports_a_stable_id() {
    let mut mock = common::spawn_mock(SCENARIO);
    let (channel, _cookie) = common::authenticated_channel(&mut mock).await;

    let first = fetch_instance_id(channel.clone()).await;
    let second = fetch_instance_id(channel).await;

    assert_eq!(
        first, second,
        "one process must report one instance id, or clients would see phantom restarts"
    );
}

#[tokio::test]
async fn restarted_process_reports_a_different_id() {
    let mut mock = common::spawn_mock(SCENARIO);
    let (channel, _cookie) = common::authenticated_channel(&mut mock).await;
    let first = fetch_instance_id(channel).await;

    // Kill the child; authenticated_channel() detects the death and respawns
    // a fresh process from the same mockfs, simulating an app restart.
    _ = mock.child.kill();
    _ = mock.child.wait();

    let (channel, _cookie) = common::authenticated_channel(&mut mock).await;
    let second = fetch_instance_id(channel).await;

    assert_ne!(
        first, second,
        "a new process must report a new instance id, or clients could not detect restarts"
    );
}
