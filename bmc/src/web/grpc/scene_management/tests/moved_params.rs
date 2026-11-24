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

//! A move that sends no params keeps params a newer package refuses as stored.

use super::params_version::{add_small_widget, fixture, update_request};
use super::update_widget;
use crate::scene;
use crate::test_support::manifest::param_key;
use bmc_grpc::web;
use bmc_widget_manifest::ParamValue;

#[tokio::test]
async fn a_move_without_params_keeps_params_the_manifest_refuses() {
    let (_temp, fixture) = fixture().await;
    let widget = add_small_widget(&fixture, "old").await;
    // Saved by an older package: it predates `label` and still has a param since retired.
    let predating = std::collections::BTreeMap::from([(
        param_key("retired"),
        ParamValue::String("kept".into()),
    )]);
    let older = semver::Version::new(0, 9, 0);
    {
        let mut config = fixture.config.write().await;
        let stored = config
            .scenes_mut()
            .get_mut(&fixture.scene_id)
            .and_then(|scene| scene.widgets.get_mut(&widget.id))
            .expect("BUG: the fixture widget was just added");
        stored.params = predating.clone();
        stored.params_version = Some(older.clone());
    }

    update_widget(
        &fixture.service,
        web::UpdateWidgetRequest {
            position: web::WidgetPosition { row: 1, col: 2 }.into(),
            params: None.into(),
            credential_bindings: None.into(),
            ..update_request(&fixture, &widget, "unused")
        },
    )
    .await
    .expect("BUG: a move must not re-check params it leaves alone");

    let moved = fixture.config.read().await.scenes()[&fixture.scene_id].widgets[&widget.id].clone();
    assert_eq!(moved.position, scene::WidgetPosition { row: 1, col: 2 });
    assert_eq!(
        (moved.params, moved.params_version),
        (predating, Some(older)),
        "untouched params keep their stamp, so a migration still owed runs"
    );
    fixture.coordinator.shutdown_widget_manager().await;
}
