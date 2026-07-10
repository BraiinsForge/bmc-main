// Copyright (C) 2026  Braiins Systems s.r.o.

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
