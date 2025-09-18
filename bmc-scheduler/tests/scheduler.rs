// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::{JobConfig, Schedule, Task};
use bmc_shared_time::time::Timezone;
use croner::Cron;
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
        (time_till_next_job_before_update.expect("BUG: ") - time_till_next_job.expect("BUG: "))
            .as_secs()
            <= 5
    );

    // Verify the cron pattern is still the same (2:30 AM daily)
    if let Some(ref schedule) = updated_job.schedule {
        assert!(schedule.pattern.as_str().contains("30 2 *"));
    }

    Ok(())
}
