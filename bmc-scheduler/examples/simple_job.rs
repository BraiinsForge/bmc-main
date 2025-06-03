// Copyright (C) 2025  Braiins Systems s.r.o.

// For more examples see: https://github.com/mvniekerk/tokio-cron-scheduler/blob/main/examples/lib.rs

use bmc_scheduler::JobId;
use bmc_scheduler::{JobScheduler, JobSchedulerLocked};
use bmc_shared::time::Timezone;
use croner::Cron;
use std::future::Future;
use std::pin::Pin;
use tokio_cron_scheduler::JobSchedulerError;

fn job_callback(
    _job_id: JobId,
    _job_scheduler: JobSchedulerLocked,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        println!("I run async every 1 hour");
    })
}

#[tokio::main]
async fn main() -> Result<(), JobSchedulerError> {
    let job_scheduler = JobSchedulerLocked::new().await?;
    let (_, rx) = tokio::sync::watch::channel(Timezone::default());
    let job_scheduler = JobScheduler::new(job_scheduler, rx);
    job_scheduler.init().await?;

    // Actual job definition
    let cron = Cron::new("0 0 * * * *")
        .with_seconds_required()
        .parse()
        .unwrap();
    let timezone = chrono_tz::Europe::Prague;
    let job_id = job_scheduler
        .submit_job_simple(cron, timezone, "display".to_owned(), job_callback)
        .await?;

    let job_details = job_scheduler.get_job(&job_id).await?;
    println!("Job details: {job_details:?}");

    Ok(())
}
