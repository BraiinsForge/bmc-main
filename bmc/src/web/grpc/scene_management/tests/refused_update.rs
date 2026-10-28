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

//! A refused update leaves the scenes as they were, dirty flag included.

use tokio::sync::broadcast::error::TryRecvError;

use super::params_version::{add_small_widget, fixture, update_request};
use super::update_widget;
use bmc_grpc::web;

#[tokio::test]
async fn a_refused_update_leaves_no_scene_change_for_the_next_save() {
    let (_temp, fixture) = fixture().await;
    let widget = add_small_widget(&fixture, "old").await;
    let mut scenes_rx = fixture.config.read().await.subscribe_scenes_change();

    let error = update_widget(
        &fixture.service,
        web::UpdateWidgetRequest {
            position: web::WidgetPosition { row: 99, col: 0 }.into(),
            ..update_request(&fixture, &widget, "new")
        },
    )
    .await
    .expect_err("BUG: row 99 is off the grid");
    assert_eq!(error.code, connectrpc::ErrorCode::InvalidArgument);

    fixture
        .config
        .write()
        .await
        .save()
        .await
        .expect("BUG: save the unchanged config");
    assert!(
        matches!(scenes_rx.try_recv(), Err(TryRecvError::Empty)),
        "an unrelated save after a refused update must not announce the scenes as changed"
    );
    fixture.coordinator.shutdown_widget_manager().await;
}
