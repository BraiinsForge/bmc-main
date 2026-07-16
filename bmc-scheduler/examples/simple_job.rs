// Copyright (C) 2025  Braiins Systems s.r.o.
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

// For more examples see: https://github.com/mvniekerk/tokio-cron-scheduler/blob/main/examples/lib.rs

use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::{JobConfig, Schedule, Task};
use bmc_shared_time::time::Timezone;
use croner::Cron;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;

fn job_callback() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        println!("I run async every 1 hour");
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (_, rx) = tokio::sync::watch::channel(Timezone::default());
    let temp_crontab =
        tempfile::NamedTempFile::new().expect("BUG: Failed to create temporary crontab file");

    let job_scheduler = JobScheduler::init(rx, Some(PathBuf::from(temp_crontab.path()))).await;
    let cron = Cron::from_str("0 0 * * * *").expect("BUG: Invalid cron expression");

    // Actual job definition
    let schedule = Schedule::Cron(cron);
    let task = Task::Async(Box::new(job_callback));
    let job_config = JobConfig::new("Test job").persist();
    let job_id = job_scheduler.schedule(schedule, task, job_config).await?;
    println!("Job id: {job_id}");
    let job_details = job_scheduler
        .job(&job_id)
        .await?
        .expect("BUG: job not found");
    assert_eq!(job_details.source, "Test job");

    // Alternative way to schedule a job
    let job_id = JobScheduler::new_job()
        .cron(Cron::from_str("* * * * *").expect("BUG: wrong cron expr"))
        .source("Test job2")
        .task(job_callback)
        .schedule(&job_scheduler)
        .await?;
    println!("Job id: {job_id}");
    let job_details = job_scheduler
        .job(&job_id)
        .await?
        .expect("BUG: job not found");
    assert_eq!(job_details.source, "Test job2");

    Ok(())
}
