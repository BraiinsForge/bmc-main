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
use crate::system_upgrade::widget_pause::test_support::{
    Call, ScriptedLifecycle, StopBehaviour, settle,
};
use axum::Router;
use axum::routing::get;
use std::future::IntoFuture;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Serves an empty image and counts the requests that arrive
/// while the scripted widgets are still running.
struct ImageServer {
    requests: Arc<AtomicUsize>,
    requests_beside_widgets: Arc<AtomicUsize>,
}

impl ImageServer {
    async fn serve(widgets: &Arc<ScriptedLifecycle>) -> (Self, String) {
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_beside_widgets = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/upgrade.img",
            get({
                let widgets = Arc::clone(widgets);
                let requests = Arc::clone(&requests);
                let requests_beside_widgets = Arc::clone(&requests_beside_widgets);
                move || async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    if widgets.running() {
                        requests_beside_widgets.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("BUG: a loopback listener binds");
        let address = listener
            .local_addr()
            .expect("BUG: a bound listener has an address");
        tokio::spawn(axum::serve(listener, router).into_future());
        let server = Self {
            requests,
            requests_beside_widgets,
        };
        (server, format!("http://{address}/upgrade.img"))
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn requests_beside_widgets(&self) -> usize {
        self.requests_beside_widgets.load(Ordering::SeqCst)
    }
}

struct SelfManaged {
    service: SystemUpgradeService<StubIndex, StubManager>,
    widgets: Arc<ScriptedLifecycle>,
    image: ImageServer,
    image_url: String,
    _image_dir: tempfile::TempDir,
    _timezone: watch::Sender<Timezone>,
}

async fn self_managed(stop: StopBehaviour) -> SelfManaged {
    let widgets = ScriptedLifecycle::new(stop);
    let (image, image_url) = ImageServer::serve(&widgets).await;
    let image_dir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let (timezone, timezone_receiver) = watch::channel(Timezone::default());
    let scheduler = JobScheduler::init(timezone_receiver, None).await;
    let hardware_capabilities = capabilities(Product::Bmc100);
    assert!(
        !hardware_capabilities.boser_managed,
        "BUG: these tests drive the local firmware run of a self-managed board"
    );
    let service = SystemUpgradeService::new(
        StubIndex,
        &image_dir.path().join("upgrade.img"),
        Arc::new(StubManager),
        StateService::new(),
        scheduler,
        tokio::time::Instant::now(),
        hardware_capabilities,
        Arc::new(StubBackend(None)),
        Arc::clone(&widgets) as Arc<dyn WidgetLifecycle>,
        PathBuf::from("/nonexistent/pending-install"),
    );
    SelfManaged {
        service,
        widgets,
        image,
        image_url,
        _image_dir: image_dir,
        _timezone: timezone,
    }
}

impl SelfManaged {
    async fn start_firmware_upgrade(&self) -> UpgradeRunStream {
        let mut detail = test_upgrade_detail();
        detail.latest_release.url = self.image_url.clone();
        let id = firmware_offer_of(&self.service.system_upgrades, detail).await;
        self.service.start_upgrade(id.to_string()).await
    }

    fn assert_image_requested_after_widgets_stopped(&self, runs: usize) {
        assert_eq!(
            self.image.requests_beside_widgets(),
            0,
            "the image lands on tmpfs; it must not be requested next to running widgets"
        );
        assert_eq!(self.image.requests(), runs);
    }
}

/// The served image cannot match the offer's hash, so a run that reached
/// the download fails there, never with the listener's `UpgradeFailed`.
fn assert_download_attempted(last: Option<UpgradeRunState>) {
    let Some(UpgradeRunState::Failed(error)) = last else {
        panic!("BUG: the served image must fail verification, got {last:?}");
    };
    assert_ne!(
        error,
        SystemUpgradeError::UpgradeFailed,
        "the run must have reached the download"
    );
}

#[tokio::test]
async fn the_download_waits_until_widgets_are_stopped() {
    let lab = self_managed(StopBehaviour::Held).await;

    let mut run = lab.start_firmware_upgrade().await;
    lab.widgets.wait_for_calls(&[Call::Stop]).await;
    assert!(
        futures::poll!(run.next()).is_pending(),
        "no download may be announced next to running widgets"
    );

    lab.widgets.release_stop();
    assert_eq!(
        run.next().await,
        Some(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading))
    );
    assert_download_attempted(drain(run).await);
    lab.assert_image_requested_after_widgets_stopped(1);
}

#[tokio::test]
async fn a_dead_widget_listener_fails_the_run_before_downloading() {
    let mut lab = self_managed(StopBehaviour::Immediate).await;
    lab.service.widget_pause = watch::channel(None).1;

    let run = lab.start_firmware_upgrade().await;

    assert_eq!(
        drain(run).await,
        Some(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed))
    );
    assert_eq!(lab.image.requests(), 0);
}

#[tokio::test]
async fn a_run_whose_pause_another_generation_took_over_fails_before_downloading() {
    let lab = self_managed(StopBehaviour::Held).await;
    let run = lab.start_firmware_upgrade().await;
    lab.widgets.wait_for_calls(&[Call::Stop]).await;
    let own = lab
        .service
        .display_state_service
        .subscribe()
        .borrow()
        .as_ref()
        .expect("BUG: the run published its snapshot")
        .generation;

    lab.service
        .display_state_service
        .publish(UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(own.get() + 1),
            state: UpgradeDisplayState::Running {
                kind: UpgradeKind::Firmware,
                phase: Some(UpgradePhase::FirmwareDownloading),
                progress: None,
            },
        });
    lab.widgets.release_stop();

    assert_eq!(
        drain(run).await,
        Some(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed)),
        "widgets stopped for another run are no licence to download beside it"
    );
    assert_eq!(lab.image.requests(), 0);
}

#[tokio::test]
async fn a_failed_run_restarts_widgets_once() {
    let lab = self_managed(StopBehaviour::Immediate).await;

    assert_download_attempted(drain(lab.start_firmware_upgrade().await).await);
    lab.widgets
        .wait_for_calls(&[Call::Stop, Call::Restart])
        .await;
    settle().await;

    assert_eq!(lab.widgets.calls(), [Call::Stop, Call::Restart]);
}

#[tokio::test]
async fn a_retry_downloads_only_after_its_own_pause() {
    let lab = self_managed(StopBehaviour::Held).await;
    let first = lab.start_firmware_upgrade().await;
    lab.widgets.wait_for_calls(&[Call::Stop]).await;
    lab.widgets.release_stop();
    assert_download_attempted(drain(first).await);
    lab.widgets
        .wait_for_calls(&[Call::Stop, Call::Restart])
        .await;

    let mut retry = lab.start_firmware_upgrade().await;
    lab.widgets
        .wait_for_calls(&[Call::Stop, Call::Restart, Call::Stop])
        .await;
    assert!(
        futures::poll!(retry.next()).is_pending(),
        "the first run's acknowledgement must not let the retry download"
    );
    assert_eq!(lab.widgets.calls(), [Call::Stop, Call::Restart, Call::Stop]);

    lab.widgets.release_stop();
    assert_eq!(
        retry.next().await,
        Some(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading))
    );
    assert_download_attempted(drain(retry).await);
    lab.assert_image_requested_after_widgets_stopped(2);
}
