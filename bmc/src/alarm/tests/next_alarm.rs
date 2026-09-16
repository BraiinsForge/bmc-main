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

fn na(fire: i64, name: &str) -> NextAlarm {
    NextAlarm {
        fire_at_utc_ms: fire,
        name: name.to_owned(),
    }
}

#[test]
fn pick_soonest_returns_none_when_no_candidates() {
    assert_eq!(pick_soonest_next_alarm(None, vec![]), None);
}

#[test]
fn pick_soonest_returns_scheduler_alone_when_no_snoozes() {
    let sched = na(1000, "morning");
    assert_eq!(
        pick_soonest_next_alarm(Some(sched.clone()), vec![]),
        Some(sched)
    );
}

#[test]
fn pick_soonest_returns_snooze_alone_when_no_scheduler() {
    let snooze = na(500, "snoozed");
    assert_eq!(
        pick_soonest_next_alarm(None, vec![snooze.clone()]),
        Some(snooze)
    );
}

#[test]
fn pick_soonest_prefers_snooze_when_it_fires_first() {
    let sched = na(1000, "morning cron");
    let snooze = na(400, "snoozed");
    assert_eq!(
        pick_soonest_next_alarm(Some(sched), vec![snooze.clone()]),
        Some(snooze)
    );
}

#[test]
fn pick_soonest_prefers_scheduler_when_it_fires_first() {
    // A scheduled alarm dated before any pending snooze
    // keeps the broadcast pointing at the cron entry.
    //
    // Should not happen with current UX (snooze << next cron),
    // but the merge rule has to be order-independent.
    let sched = na(100, "morning cron");
    let snooze = na(900, "snoozed");
    assert_eq!(
        pick_soonest_next_alarm(Some(sched.clone()), vec![snooze]),
        Some(sched)
    );
}

#[test]
fn pick_soonest_picks_earliest_among_multiple_snoozes() {
    let sched = na(2000, "cron");
    let snoozes = vec![na(1500, "later snooze"), na(800, "earlier snooze")];
    assert_eq!(
        pick_soonest_next_alarm(Some(sched), snoozes),
        Some(na(800, "earlier snooze"))
    );
}

// Tests that pin the full contract of `recompute_and_broadcast_next_alarm`
// — across empty / present scheduler entries, race-window lookup misses,
// and the snooze merge — so the next refactor can't silently drop a branch.
mod recompute {
    use std::str::FromStr;

    use bmc_scheduler::{
        Cron, JobScheduler,
        scheduler::{JobConfig, Schedule, Task},
    };
    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    use super::*;

    async fn make_scheduler() -> (JobScheduler, TempDir) {
        let temp_dir = TempDir::new().expect("BUG: failed to create tempdir");
        let (_tz_tx, tz_rx) = tokio::sync::watch::channel(Timezone::default());
        let scheduler = JobScheduler::init(tz_rx, Some(temp_dir.path().join("crontab"))).await;
        (scheduler, temp_dir)
    }

    async fn schedule_alarm_job(
        scheduler: &JobScheduler,
        name: &str,
    ) -> (AlarmId, ScheduledAlarm, DateTime<Utc>) {
        let cron = Cron::from_str("0 0 12 * * *").expect("BUG: failed to parse cron");
        let task: bmc_scheduler::BoxedTask = Box::new(|| Box::pin(async {}));
        let job_id = scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Async(task),
                JobConfig::new(AlarmScheduler::SCHEDULER_SOURCE),
            )
            .await
            .expect("BUG: failed to schedule cron job");
        let next_tick = scheduler
            .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
            .await
            .expect("BUG: failed to fetch jobs by source")
            .into_iter()
            .find(|j| j.job_id == job_id)
            .and_then(|j| j.next_tick)
            .expect("BUG: scheduler left next_tick unset on the freshly scheduled job");
        (
            AlarmId::generate(),
            ScheduledAlarm {
                job_id,
                name: name.to_owned(),
            },
            next_tick,
        )
    }

    fn pending_snooze(fire_at_utc_ms: i64, name: &str) -> PendingSnooze {
        PendingSnooze {
            cancel: CancellationToken::new(),
            fire_at_utc_ms,
            name: name.to_owned(),
        }
    }

    async fn run(
        scheduler: &JobScheduler,
        active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>>,
        snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>>,
    ) -> Option<NextAlarm> {
        let (tx, rx) = tokio::sync::watch::channel::<Option<NextAlarm>>(None);
        recompute_and_broadcast_next_alarm(
            scheduler,
            AlarmScheduler::SCHEDULER_SOURCE,
            &active,
            &snoozes,
            &tx,
        )
        .await;
        rx.borrow().clone()
    }

    #[tokio::test]
    async fn empty_scheduler_and_no_snoozes_broadcasts_none() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        assert_eq!(run(&scheduler, active, snoozes).await, None);
    }

    #[tokio::test]
    async fn empty_scheduler_with_snooze_broadcasts_snooze() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        snoozes
            .lock()
            .await
            .insert(AlarmId::generate(), pending_snooze(42, "snooze"));
        assert_eq!(
            run(&scheduler, active, snoozes).await,
            Some(NextAlarm {
                fire_at_utc_ms: 42,
                name: "snooze".to_owned(),
            }),
        );
    }

    #[tokio::test]
    async fn scheduled_alarm_resolves_to_its_next_tick() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        active.lock().await.insert(alarm_id, scheduled);
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        assert_eq!(
            run(&scheduler, active, snoozes).await,
            Some(NextAlarm {
                fire_at_utc_ms: tick.timestamp_millis(),
                name: "morning".to_owned(),
            }),
        );
    }

    #[tokio::test]
    async fn scheduler_wins_when_it_fires_before_snooze() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        active.lock().await.insert(alarm_id, scheduled);
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        let later = tick.timestamp_millis() + 1_000_000;
        snoozes
            .lock()
            .await
            .insert(AlarmId::generate(), pending_snooze(later, "late snooze"));
        assert_eq!(
            run(&scheduler, active, snoozes).await,
            Some(NextAlarm {
                fire_at_utc_ms: tick.timestamp_millis(),
                name: "morning".to_owned(),
            }),
        );
    }

    #[tokio::test]
    async fn snooze_wins_when_it_fires_before_scheduler() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        active.lock().await.insert(alarm_id, scheduled);
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        let earlier = tick.timestamp_millis() - 1_000_000;
        snoozes
            .lock()
            .await
            .insert(AlarmId::generate(), pending_snooze(earlier, "early snooze"));
        assert_eq!(
            run(&scheduler, active, snoozes).await,
            Some(NextAlarm {
                fire_at_utc_ms: earlier,
                name: "early snooze".to_owned(),
            }),
        );
    }

    // Race window between `jobs_by_source` and `active_alarms` lookup:
    // scheduler still reports a job_id whose entry has just been removed from `active_alarms`.
    // A pending snooze must still drive the broadcast — silently dropping it masks the snooze on every widget.
    #[tokio::test]
    async fn race_miss_with_snooze_broadcasts_snooze() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        // Schedule a job so `jobs_by_source` returns one,
        // but don't insert it into `active_alarms` — the lookup misses.
        let _ = schedule_alarm_job(&scheduler, "ignored").await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        snoozes
            .lock()
            .await
            .insert(AlarmId::generate(), pending_snooze(42, "snooze"));
        assert_eq!(
            run(&scheduler, active, snoozes).await,
            Some(NextAlarm {
                fire_at_utc_ms: 42,
                name: "snooze".to_owned(),
            }),
        );
    }

    // Race window with no snooze: the scheduler reports a job
    // whose `active_alarms` entry has just been removed.
    // With nothing else to offer,
    // the broadcast must resolve to "no alarm".
    #[tokio::test]
    async fn race_miss_without_snooze_broadcasts_none() {
        let (scheduler, _temp_dir) = make_scheduler().await;
        let _ = schedule_alarm_job(&scheduler, "ignored").await;
        let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
        let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
        assert_eq!(run(&scheduler, active, snoozes).await, None);
    }
}
