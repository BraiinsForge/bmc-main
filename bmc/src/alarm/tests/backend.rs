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

use super::super::*;
use super::AlarmFixture;

struct BackendFixture {
    alarm: AlarmFixture,
    expected_config: Vec<u8>,
}

async fn backend_fixture() -> BackendFixture {
    let alarm = AlarmFixture::new(bmc_platform::Product::Bmm101).await;
    {
        let mut config = alarm.config_handle.write().await;
        config.add_alarm(AlarmData::new(
            true,
            "persisted".to_owned(),
            NaiveTime::from_hms_opt(7, 30, 0).expect("BUG: valid alarm test time"),
            BTreeSet::new(),
            None,
            None,
        ));
        config
            .save()
            .await
            .expect("BUG: persist alarm backend test config");
    }
    let expected_config = tokio::fs::read(&alarm.config_path)
        .await
        .expect("BUG: read alarm backend test config");

    BackendFixture {
        alarm,
        expected_config,
    }
}

#[tokio::test]
async fn unsupported_backend_preserves_enabled_alarm_without_scheduling_it() {
    let fixture = backend_fixture().await;
    let backend = fixture.alarm.init_backend(false).await;
    let next_alarm_receiver = backend.subscribe_next_alarm();

    assert!(backend.controller().is_none());
    assert!(next_alarm_receiver.borrow().is_none());
    assert!(
        next_alarm_receiver.has_changed().is_ok(),
        "unsupported alarm channel must remain open"
    );
    assert!(
        fixture
            .alarm
            .scheduler
            .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
            .await
            .expect("BUG: list alarm jobs")
            .is_empty()
    );
    let alarms = fixture.alarm.config_handle.read().await.alarms();
    assert_eq!(alarms.len(), 1);
    assert!(alarms[0].enabled);
    assert_eq!(
        tokio::fs::read(&fixture.alarm.config_path)
            .await
            .expect("BUG: reread alarm backend test config"),
        fixture.expected_config
    );
}

#[tokio::test]
async fn supported_backend_schedules_enabled_persisted_alarm() {
    let fixture = backend_fixture().await;
    let backend = fixture.alarm.init_backend(true).await;

    assert!(backend.controller().is_some());
    assert_eq!(
        fixture
            .alarm
            .scheduler
            .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
            .await
            .expect("BUG: list alarm jobs")
            .len(),
        1
    );
    assert!(backend.subscribe_next_alarm().borrow().is_some());
}
