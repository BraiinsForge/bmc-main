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
use futures::FutureExt as _;

#[tokio::test]
async fn managed_enabling_keeps_the_local_check_disabled() {
    let (service, _timezone_sender) =
        stub_service_with_capabilities(capabilities(Product::Bfm100)).await;

    service
        .apply_autoupgrade(true)
        .await
        .expect("BUG: suppressing a managed schedule must succeed");

    assert!(!service.autoupgrade_enabled.load(Ordering::SeqCst));
    let jobs = service
        .scheduler
        .jobs_by_source(AutoUpgrade::AUTOUPGRADE_SOURCE_NAME)
        .await
        .expect("BUG: the scheduler must list jobs");
    assert!(jobs.is_empty(), "managed enabling must not register a job");
}

#[tokio::test]
async fn self_managed_initialization_registers_an_automatic_upgrade_job() {
    let (service, _timezone_sender) =
        stub_service_with_capabilities(capabilities(Product::Bmc100)).await;

    service.autoupgrade_init(true).await;
    let jobs = service
        .scheduler
        .jobs_by_source(AutoUpgrade::AUTOUPGRADE_SOURCE_NAME)
        .await
        .expect("BUG: the scheduler must list jobs");
    assert_eq!(
        jobs.len(),
        1,
        "self-managed initialization must register an automatic-upgrade job"
    );
}

#[tokio::test]
async fn managed_initialization_does_not_spawn_a_trigger_listener() {
    let (service, _timezone_sender) =
        stub_service_with_capabilities(capabilities(Product::Bfm100)).await;

    service.autoupgrade_init(true).await;
    let jobs = service
        .scheduler
        .jobs_by_source(AutoUpgrade::AUTOUPGRADE_SOURCE_NAME)
        .await
        .expect("BUG: the scheduler must list jobs");
    assert!(
        jobs.is_empty(),
        "managed initialization must not register an automatic-upgrade job"
    );
    service.autoupgrade.notifier.notify_one();
    tokio::task::yield_now().await;

    assert!(
        service
            .autoupgrade
            .notifier
            .notified()
            .now_or_never()
            .is_some(),
        "a managed service must not spawn a task that consumes trigger notifications"
    );
}

#[tokio::test]
async fn immediate_checks_follow_maintenance_ownership() {
    for (product, expected_notification) in [(Product::Bmc100, true), (Product::Bfm100, false)] {
        let (service, _timezone_sender) =
            stub_service_with_capabilities(capabilities(product)).await;

        service.autoupgrade_check_now();

        assert_eq!(
            service
                .autoupgrade
                .notifier
                .notified()
                .now_or_never()
                .is_some(),
            expected_notification,
            "unexpected immediate-check behavior for {product:?}"
        );
    }
}

#[tokio::test]
async fn periodic_gc_follows_maintenance_ownership() {
    for (product, expected_jobs) in [(Product::Bmc100, 1), (Product::Bfm100, 0)] {
        let (service, _timezone_sender) =
            stub_service_with_capabilities(capabilities(product)).await;

        service
            .gc_init(PathBuf::from("/nonexistent/gc-config.json"))
            .await;

        let jobs = service
            .scheduler
            .jobs_by_source(periodic_gc::PERIODIC_GC_SOURCE)
            .await
            .expect("BUG: the scheduler must list jobs");
        assert_eq!(
            jobs.len(),
            expected_jobs,
            "unexpected GC jobs for {product:?}"
        );
    }
}
