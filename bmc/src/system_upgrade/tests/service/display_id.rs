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

use super::*;

async fn service_offering_firmware(
    backend: Arc<dyn PackageBackend>,
) -> (
    SystemUpgradeService<DiscoveryIndex, StubManager>,
    watch::Sender<Timezone>,
) {
    discovery_service(
        DiscoveryIndex(Ok(Some(vec![test_upgrade_detail().latest_release]))),
        backend,
    )
    .await
}

#[tokio::test]
async fn a_started_offer_is_displayed_under_its_id() {
    let (service, _timezone) =
        service_offering_firmware(Arc::new(StubBackend(Some(Arc::default())))).await;
    let display = service.subscribe_run_status();
    let id = firmware_offer(&service.system_upgrades).await;

    let _run = service.start_upgrade(id.to_string()).await;

    assert_eq!(
        display.borrow().as_ref().map(|snapshot| snapshot.id),
        Some(Some(id)),
        "the display names the offer a run was started from"
    );
}

#[tokio::test]
async fn an_automatic_upgrade_is_displayed_without_an_id() {
    let (service, _timezone) =
        service_offering_firmware(Arc::new(RecordingGcBackend::new([]))).await;
    let display = service.subscribe_run_status();

    let run = service
        .start_automatic_upgrade()
        .await
        .expect("BUG: the stub index offers a firmware release");
    assert!(run.is_some(), "the offered release is dispatched");

    assert_eq!(
        display.borrow().as_ref().map(|snapshot| snapshot.id),
        Some(None),
        "an automatic run is started from no offer, so no reader may take it for its own"
    );
}
