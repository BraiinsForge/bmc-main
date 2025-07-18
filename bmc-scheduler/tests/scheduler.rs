// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_scheduler::JobId;
use bmc_scheduler::scheduler::JobSchedulerError;
use bmc_scheduler::{JobScheduler, JobSchedulerLocked};
use bmc_shared_time::time::Timezone;
use chrono_tz::US::Eastern;
use croner::Cron;
use std::future::Future;
use std::pin::Pin;
use tokio::time::{Duration, sleep};

fn simple_job_callback(
    _job_id: JobId,
    _job_scheduler: JobSchedulerLocked,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        println!("Test job executed");
    })
}

#[tokio::test]
async fn test_timezone_change_and_reschedule() -> Result<(), JobSchedulerError> {
    // Create job scheduler with initial America/New_York (a.k.a US/Eastern) timezone
    let job_scheduler = JobSchedulerLocked::new().await?;
    let (timezone_sender, timezone_receiver) = tokio::sync::watch::channel(
        Timezone::list()
            .iter()
            .find(|tz| tz.iana() == "America/New_York")
            .expect("America/New_York timezone not found")
            .clone(),
    );

    let mut scheduler = JobScheduler::new(job_scheduler, timezone_receiver);
    scheduler.init().await?;

    // Submit a job scheduled for 2:30 AM daily
    let cron_schedule = Cron::new("0 30 2 * * *")
        .with_seconds_required()
        .with_dom_and_dow()
        .parse()
        .expect("Failed to parse cron schedule");

    let job_id = scheduler
        .submit_job_simple(
            cron_schedule,
            Eastern,
            "test_job".to_owned(),
            simple_job_callback,
        )
        .await?;

    // Wait for job to be registered
    sleep(Duration::from_millis(100)).await;

    // Get initial job details to verify creation
    let initial_job = scheduler.get_job(&job_id).await?.unwrap();
    assert_eq!(initial_job.source, "test_job");
    assert!(initial_job.schedule.is_some());
    let initial_next_tick = initial_job.next_tick;

    println!("Initial job next tick: {initial_next_tick:?}");

    // Change timezone to Europe/Prague
    let new_timezone = Timezone::list()
        .iter()
        .find(|tz| tz.iana() == "Europe/Prague")
        .expect("Europe/Prague timezone not found")
        .clone();
    let time_till_next_job_before_update = scheduler.time_till_next_job().await?;
    println!("Time till next job before update: {time_till_next_job_before_update:?}");

    timezone_sender.send(new_timezone).unwrap();

    // Wait for timezone change to be processed
    sleep(Duration::from_millis(200)).await;

    // Get updated job details after timezone change
    let updated_job = scheduler.get_job(&job_id).await?.unwrap();

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
    assert!(time_till_next_job_before_update.unwrap() > time_till_next_job.unwrap());

    // Verify the cron pattern is still the same (2:30 AM daily)
    if let Some(ref schedule) = updated_job.schedule {
        assert!(schedule.pattern.as_str().contains("30 2 *"));
    }

    Ok(())
}
