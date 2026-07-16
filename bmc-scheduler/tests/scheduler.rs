// Copyright (C) 2025  Braiins Systems s.r.o.
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

use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::{JobConfig, Schedule, Task};
use bmc_shared_time::time::Timezone;
use croner::Cron;
use rand::prelude::IndexedRandom;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use tokio::time::{Duration, sleep};

fn simple_job_callback() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        println!("Test job executed");
    })
}

#[tokio::test]
async fn test_timezone_change_and_reschedule() -> anyhow::Result<()> {
    // Create job scheduler with initial US/Eastern timezone
    let timezone = Timezone::from_str("America/Detroit").expect("BUG: Failed to parse timezone");
    let (timezone_sender, timezone_receiver) = tokio::sync::watch::channel(timezone.clone());
    let temp_crontab =
        tempfile::NamedTempFile::new().expect("BUG: Failed to create temporary crontab file");

    let mut scheduler =
        JobScheduler::init(timezone_receiver, Some(PathBuf::from(temp_crontab.path()))).await;

    // Submit a job scheduled for 2:30 AM daily
    let cron_schedule = Cron::from_str("0 30 2 * * *").expect("BUG: Failed to parse cron schedule");

    let schedule = Schedule::Cron(cron_schedule);

    let task = Task::Async(Box::new(simple_job_callback));

    let job_config = JobConfig {
        source: "test_job".to_owned(),
        persist_to_crontab: true,
    };

    let job_id = scheduler
        .schedule(schedule, task, job_config)
        .await
        .expect("BUG: Failed to schedule job");

    // Wait for job to be registered
    sleep(Duration::from_millis(100)).await;

    // Get initial job details to verify creation
    let initial_job = scheduler
        .job(&job_id)
        .await?
        .expect("BUG: Failed to get job");
    assert_eq!(initial_job.source, "test_job");
    assert!(initial_job.schedule.is_some());
    let initial_next_tick = initial_job.next_tick;

    println!("Initial job next tick: {initial_next_tick:?}");

    // Change timezone to Europe/Prague
    let new_timezone = Timezone::from_str("Europe/Prague").expect("BUG: Failed to parse timezone");
    let time_till_next_job_before_update = scheduler.time_till_next_job().await?;
    println!("Time till next job before update: {time_till_next_job_before_update:?}");

    timezone_sender
        .send(new_timezone)
        .expect("BUG: Failed to send timezone change notification");

    // Wait for timezone change to be processed
    sleep(Duration::from_millis(200)).await;

    // Get updated job details after timezone change
    let updated_job = scheduler
        .job(&job_id)
        .await?
        .expect("BUG: Failed to get job");

    // Validate timezone change worked
    assert_eq!(updated_job.source, "test_job");
    assert!(updated_job.schedule.is_some());

    // Validate that the rescheduled time is reflected
    let updated_next_tick = updated_job.next_tick;
    println!("Updated job next tick: {updated_next_tick:?}");

    // The next tick should be different after timezone change (unless coincidentally same UTC time)
    // Since we're changing from US/Eastern to Europe/Prague, the timing will be different
    assert!(updated_next_tick.is_some());
    let time_till_next_job = scheduler.time_till_next_job().await?;
    println!("Time till next job: {time_till_next_job:?}");
    assert!(time_till_next_job_before_update.is_some());
    assert!(time_till_next_job.is_some());
    assert_ne!(
        time_till_next_job_before_update.expect("BUG: "),
        time_till_next_job.expect("BUG: ")
    );

    // Verify the cron pattern is still the same (2:30 AM daily)
    if let Some(ref schedule) = updated_job.schedule {
        assert!(schedule.pattern.as_str().contains("30 2 *"));
    }

    Ok(())
}

#[tokio::test]
async fn test_timezone_change_and_reschedule_oneshot() -> anyhow::Result<()> {
    // Create job scheduler with initial US/Eastern timezone
    let timezone = Timezone::from_str("America/Detroit").expect("BUG: Failed to parse timezone");
    let (timezone_sender, timezone_receiver) = tokio::sync::watch::channel(timezone.clone());
    let temp_crontab =
        tempfile::NamedTempFile::new().expect("BUG: Failed to create temporary crontab file");

    let mut scheduler =
        JobScheduler::init(timezone_receiver, Some(PathBuf::from(temp_crontab.path()))).await;

    // Submit one-shot job
    let schedule = Schedule::OneShot(Duration::from_secs(212121));
    let task = Task::Async(Box::new(simple_job_callback));
    let job_config = JobConfig {
        source: "test_job".to_owned(),
        persist_to_crontab: true,
    };

    let job_id = scheduler
        .schedule(schedule, task, job_config)
        .await
        .expect("BUG: Failed to schedule job");

    // Wait for job to be registered
    sleep(Duration::from_millis(100)).await;

    // Get initial job details to verify creation
    let initial_job = scheduler
        .job(&job_id)
        .await?
        .expect("BUG: Failed to get job");
    assert_eq!(initial_job.source, "test_job");
    assert!(initial_job.schedule.is_none());
    let initial_next_tick = initial_job.next_tick;

    println!("Initial job next tick: {initial_next_tick:?}");

    // Change timezone to Europe/Prague
    let new_timezone = Timezone::from_str("Europe/Prague").expect("BUG: Failed to parse timezone");
    let time_till_next_job_before_update = scheduler.time_till_next_job().await?;
    println!("Time till next job before update: {time_till_next_job_before_update:?}");

    timezone_sender
        .send(new_timezone)
        .expect("BUG: Failed to send timezone change notification");

    // Wait for timezone change to be processed
    sleep(Duration::from_millis(200)).await;
    let time_till_next_job_after_update = scheduler.time_till_next_job().await?;
    println!("Time till next job after update: {time_till_next_job_after_update:?}");

    // Get updated job details after timezone change
    let updated_job = scheduler
        .job(&job_id)
        .await?
        .expect("BUG: Failed to get job");

    // Validate timezone change worked
    assert_eq!(updated_job.source, "test_job");
    assert!(updated_job.schedule.is_none());

    // Validate that the rescheduled time is reflected
    let updated_next_tick = updated_job.next_tick;
    println!("Updated job next tick: {updated_next_tick:?}");

    // The next tick should be different after timezone change (unless coincidentally same UTC time)
    // Since we're changing from US/Eastern to Europe/Prague, the timing will be different
    assert!(updated_next_tick.is_some());
    let time_till_next_job = scheduler.time_till_next_job().await?;
    println!("Time till next job: {time_till_next_job:?}");
    assert!(time_till_next_job_before_update.is_some());
    assert!(time_till_next_job.is_some());
    assert!(
        time_till_next_job_before_update
            .expect("BUG: ")
            .checked_sub(time_till_next_job.expect("BUG: "))
            .expect("BUG: checked_sub")
            .as_secs()
            <= 5
    );

    // Verify the cron pattern is still the same (2:30 AM daily)
    if let Some(ref schedule) = updated_job.schedule {
        assert!(schedule.pattern.as_str().contains("30 2 *"));
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(clippy::too_many_lines)]
async fn test_high_concurrent_stress_deadlock_detection() -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    const WORKER_COUNT: usize = 100;
    const OPERATIONS_PER_WORKER: usize = 100;
    const TEST_DURATION_SECS: u64 = 5;
    const OVERALL_TIMEOUT_SECS: u64 = 30; // Add overall timeout to catch deadlocks

    // Create scheduler
    let timezone = Timezone::default();
    let (timezone_sender, timezone_receiver) = tokio::sync::watch::channel(timezone.clone());
    let temp_crontab =
        tempfile::NamedTempFile::new().expect("BUG: Failed to create temporary crontab file");
    let scheduler =
        JobScheduler::init(timezone_receiver, Some(PathBuf::from(temp_crontab.path()))).await;

    // Statistics tracking
    let scheduled_count = Arc::new(AtomicUsize::new(0));
    let cancelled_count = Arc::new(AtomicUsize::new(0));
    let query_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));

    // Shared state for scheduled jobs
    let active_jobs = Arc::new(tokio::sync::Mutex::new(Vec::<uuid::Uuid>::new()));

    println!(
        "🔥 Starting scheduler stress test with {WORKER_COUNT} workers, {OPERATIONS_PER_WORKER} ops each"
    );

    let start_time = Instant::now();
    let mut handles = Vec::new();

    // Spawn timezone changer task (adds more chaos)
    let timezone_changer = {
        let timezone_sender = timezone_sender.clone();
        tokio::spawn(async move {
            let timezones = [
                "UTC",
                "America/New_York",
                "Europe/London",
                "Asia/Tokyo",
                "Australia/Sydney",
                "America/Los_Angeles",
                "Europe/Berlin",
            ];

            loop {
                let sleep_duration = {
                    use rand::Rng;
                    let mut rng = rand::rng();
                    Duration::from_millis(rng.random_range(50..500))
                };
                sleep(sleep_duration).await;

                let tz_str = {
                    let mut rng = rand::rng();
                    *timezones.choose(&mut rng).expect("BUG: ")
                };

                if let Ok(tz) = Timezone::from_str(tz_str) {
                    let _ = timezone_sender.send(tz);
                }

                if start_time.elapsed() > Duration::from_secs(TEST_DURATION_SECS) {
                    break;
                }
            }
        })
    };

    // Wrap entire test in a timeout
    let test_result = tokio::time::timeout(Duration::from_secs(OVERALL_TIMEOUT_SECS), async {
        // Spawn worker tasks
        for worker_id in 0..WORKER_COUNT {
            let scheduler = scheduler.clone();
            let active_jobs = active_jobs.clone();
            let scheduled_count = scheduled_count.clone();
            let cancelled_count = cancelled_count.clone();
            let query_count = query_count.clone();
            let error_count = error_count.clone();

            let handle = tokio::spawn(async move {
                use rand::prelude::*;
                let cron_patterns = [
                    "0 0 * * *",         // Daily
                    "*/5 * * * *",       // Every 5 minutes
                    "0 */2 * * *",       // Every 2 hours
                    "30 14 * * 1-5",     // Weekdays 2:30 PM
                    "0 0 1 * *",         // Monthly
                    "*/10 9-17 * * 1-5", // Business hours
                ];

                for op_id in 0..OPERATIONS_PER_WORKER {
                    if start_time.elapsed() > Duration::from_secs(TEST_DURATION_SECS) {
                        break;
                    }

                    // Generate random values without holding RNG across await points
                    let operation = {
                        let mut rng = rand::rng();
                        rng.random_range(0..100)
                    };

                    match operation {
                        // 40% - Schedule new jobs (mix of cron and oneshot)
                        0..40 => {
                            let (job_type, persist, pattern_idx, delay, cmd_idx) = {
                                let mut rng = rand::rng();
                                (
                                    rng.random_range(0..3),
                                    rng.random_bool(0.3),
                                    rng.random_range(0..cron_patterns.len()),
                                    rng.random_range(100..10000),
                                    rng.random_range(0..2),
                                )
                            };

                            let config = JobConfig {
                                source: format!("worker_{worker_id}_op_{op_id}"),
                                persist_to_crontab: persist,
                            };

                            let schedule_result = match job_type {
                                0 => {
                                    // Cron job with async task
                                    let pattern = cron_patterns[pattern_idx];
                                    let cron = Cron::from_str(pattern);
                                    if let Ok(cron) = cron {
                                        let task = Task::Async(Box::new(simple_job_callback));
                                        scheduler.schedule(Schedule::Cron(cron), task, config).await
                                    } else {
                                        continue;
                                    }
                                }
                                1 => {
                                    // OneShot with async task
                                    let task = Task::Async(Box::new(simple_job_callback));
                                    scheduler
                                        .schedule(
                                            Schedule::OneShot(Duration::from_millis(delay)),
                                            task,
                                            config,
                                        )
                                        .await
                                }
                                _ => {
                                    // Command task
                                    let commands = ["true", "true"];
                                    let cmd = commands[cmd_idx];
                                    let task = Task::Command(cmd.to_owned());
                                    scheduler
                                        .schedule(
                                            Schedule::OneShot(Duration::from_millis(delay)),
                                            task,
                                            config,
                                        )
                                        .await
                                }
                            };

                            match schedule_result {
                                Ok(job_id) => {
                                    scheduled_count.fetch_add(1, Ordering::Relaxed);
                                    // Add to active jobs list for potential cancellation
                                    {
                                        let mut jobs = active_jobs.lock().await;
                                        jobs.push(job_id);
                                        // Keep list manageable
                                        if jobs.len() > 200 {
                                            jobs.drain(0..50);
                                        }
                                    }
                                }
                                Err(_) => {
                                    error_count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }

                        // 25% - Cancel random jobs
                        40..65 => {
                            let job_to_cancel = {
                                let mut jobs = active_jobs.lock().await;
                                if jobs.is_empty() {
                                    None
                                } else {
                                    let idx = {
                                        let mut rng = rand::rng();
                                        rng.random_range(0..jobs.len())
                                    };
                                    Some(jobs.swap_remove(idx))
                                }
                            };

                            if let Some(job_id) = job_to_cancel {
                                if scheduler.cancel(&job_id).await.is_ok() {
                                    cancelled_count.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    error_count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }

                        // 15% - Query individual jobs
                        65..80 => {
                            let job_to_query = {
                                let jobs = active_jobs.lock().await;
                                if jobs.is_empty() {
                                    None
                                } else {
                                    let idx = {
                                        let mut rng = rand::rng();
                                        rng.random_range(0..jobs.len())
                                    };
                                    Some(jobs[idx])
                                }
                            };

                            if let Some(job_id) = job_to_query {
                                let _ = scheduler.job(&job_id).await;
                                query_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        // 10% - List all jobs (expensive operation)
                        80..90 => {
                            if scheduler.jobs().await.is_ok() {
                                query_count.fetch_add(1, Ordering::Relaxed);
                            } else {
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        // 5% - Cancel jobs by source (batch operation)
                        90..95 => {
                            let target_worker = {
                                let mut rng = rand::rng();
                                rng.random_range(0..WORKER_COUNT)
                            };
                            let source = format!("worker_{target_worker}");
                            scheduler.cancel_jobs(source).await;
                            cancelled_count.fetch_add(1, Ordering::Relaxed);
                        }

                        // 5% - Time till next job (requires both locks)
                        _ => {
                            if scheduler.clone().time_till_next_job().await.is_ok() {
                                query_count.fetch_add(1, Ordering::Relaxed);
                            } else {
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    // Random short delay to create timing variations
                    let should_delay = {
                        let mut rng = rand::rng();
                        rng.random_bool(0.3)
                    };
                    if should_delay {
                        let delay_ms = {
                            let mut rng = rand::rng();
                            rng.random_range(1..10)
                        };
                        sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all workers to complete or timeout
        let timeout_duration = Duration::from_secs(TEST_DURATION_SECS + 10);
        let start = Instant::now();

        for handle in handles {
            let remaining_time = timeout_duration.saturating_sub(start.elapsed());
            if remaining_time.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining_time, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("Worker panicked: {e:?}"),
                Err(_) => eprintln!("Worker timed out"),
            }
        }

        // Stop timezone changer
        timezone_changer.abort();

        // Final statistics
        let total_scheduled = scheduled_count.load(Ordering::Relaxed);
        let total_cancelled = cancelled_count.load(Ordering::Relaxed);
        let total_queries = query_count.load(Ordering::Relaxed);
        let total_errors = error_count.load(Ordering::Relaxed);
        let elapsed = start_time.elapsed();

        println!("🎯 Scheduler stress test completed in {elapsed:?}");
        println!("📊 Stats:");
        println!("   - Jobs scheduled: {total_scheduled}");
        println!("   - Jobs cancelled: {total_cancelled}");
        println!("   - Queries performed: {total_queries}");
        println!("   - Errors encountered: {total_errors}");
        #[expect(clippy::cast_precision_loss)]
        let ops_per_sec =
            (total_scheduled + total_cancelled + total_queries) as f64 / elapsed.as_secs_f64();
        println!("   - Operations/sec: {ops_per_sec:.2}");

        // Final verification - scheduler should still be responsive
        let final_jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Scheduler should still be responsive after stress test");
        let final_jobs_count = final_jobs.len();
        println!("   - Final active jobs: {final_jobs_count}");

        // Test should pass if:
        // 1. No deadlocks occurred (we reached this point)
        // 2. Scheduler is still responsive
        // 3. We performed significant work
        assert!(
            total_scheduled > 50,
            "Should have scheduled substantial number of jobs"
        );
        assert!(
            total_queries > 20,
            "Should have performed substantial queries"
        );
        assert!(
            total_errors * 10 < total_scheduled,
            "Error rate should be reasonable"
        );

        println!("✅ No deadlocks detected! Scheduler survived stress test.");

        Ok(())
    })
    .await;

    match test_result {
        Ok(Ok(())) => {
            // Test completed successfully within the timeout
            println!("✅✅ Test completed successfully within the timeout!");
            Ok(())
        }
        Ok(Err(e)) => {
            // Test failed
            eprintln!("❌❌ Test failed: {e:?}");
            Err(e)
        }
        Err(_) => {
            // Timeout occurred - DEADLOCK likely
            eprintln!("❌❌❌ Timeout occurred! Suspected DEADLOCK!");
            panic!("Timeout occurred! Suspected DEADLOCK!");
        }
    }
}
