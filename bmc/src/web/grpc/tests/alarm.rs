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

//! The alarm service is mounted only on hardware with an alarm controller.

use std::sync::Arc;

use bmc_grpc::web::{AddAlarmRequest, AlarmServiceClient, SetAlarmEnabledRequest, SetAlarmRequest};
use bmc_platform::{HardwareProfile, Product};
use buffa_types::google::protobuf::{Empty, StringValue};
use connectrpc::{ConnectError, ErrorCode, Router};

use super::super::{alarm::AlarmService, gated};
use super::{grpc, grpc_web, transport};
use crate::alarm::{AlarmController, tests::supported_alarm_controller};
use crate::test_support::StubSessionManager;

fn alarm_router(alarm_controller: Option<AlarmController>) -> axum::Router {
    let mut router = Router::new();
    if let Some(alarm_controller) = alarm_controller {
        router = router.add_service(Arc::new(AlarmService::new(alarm_controller)));
    }
    gated(
        router,
        Arc::new(StubSessionManager),
        HardwareProfile::for_product(Product::Bmc100).capabilities(),
    )
}

#[tokio::test]
async fn unregistered_alarm_service_methods_are_unimplemented() {
    let client = AlarmServiceClient::new(transport(alarm_router(None)), grpc_web());

    let results: Vec<(&str, Result<(), ConnectError>)> = vec![
        (
            "GetAlarmInfo",
            client.get_alarm_info(Empty::default()).await.map(|_| ()),
        ),
        (
            "ListAlarms",
            client.list_alarms(Empty::default()).await.map(|_| ()),
        ),
        (
            "AddAlarm",
            client
                .add_alarm(AddAlarmRequest::default())
                .await
                .map(|_| ()),
        ),
        (
            "SetAlarm",
            client
                .set_alarm(SetAlarmRequest::default())
                .await
                .map(|_| ()),
        ),
        (
            "DeleteAlarm",
            client
                .delete_alarm(StringValue::default())
                .await
                .map(|_| ()),
        ),
        (
            "SetAlarmEnabled",
            client
                .set_alarm_enabled(SetAlarmEnabledRequest::default())
                .await
                .map(|_| ()),
        ),
    ];

    for (method, result) in results {
        let error = result.expect_err("an unmounted alarm method answers with an error");
        assert_eq!(error.code, ErrorCode::Unimplemented, "{method}");
    }
}

#[tokio::test]
async fn registered_alarm_service_is_behind_authentication() {
    let (_temp, _timezone_sender, alarm_controller) =
        supported_alarm_controller(Product::Bmc100).await;
    let router = alarm_router(Some(alarm_controller));

    for (protocol, config) in [("gRPC", grpc()), ("gRPC-Web", grpc_web())] {
        let error = AlarmServiceClient::new(transport(router.clone()), config)
            .get_alarm_info(Empty::default())
            .await
            .expect_err("a call without a session is refused");

        assert_eq!(error.code, ErrorCode::Unauthenticated, "{protocol}");
        assert_eq!(
            error.message.as_deref(),
            Some("Missing or invalid session"),
            "{protocol}"
        );
    }
}
