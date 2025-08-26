// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::Cron;
use chrono::{DateTime, Utc};
use std::pin::Pin;
use std::sync::Arc;
pub use tokio_cron_scheduler::{Context as JobContext, Job, job::JobBuilder, job::JobId};

// NOTE: BoxedTask represents a function that returns a Future. Future holds the main logic
// for the task
pub type BoxedTask = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[must_use]
pub fn to_boxed(task: Arc<BoxedTask>) -> BoxedTask {
    Box::new(move || (task)())
}

#[derive(Clone)]
pub struct JobDetails {
    pub job_id: JobId,
    pub job: Job,
    pub schedule: Option<Cron>,
    pub next_tick: Option<DateTime<Utc>>,
    /// Source of the job, e.g. "display", "upgrade", "notification"
    pub source: String,
    pub command: Option<String>,
}

impl std::fmt::Debug for JobDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "JobDetails {{ id: {:?}, source: {:?}, command: {:?}, schedule: {:?}, next_tick: {:?} }}",
            self.job_id, self.source, self.command, self.schedule, self.next_tick
        )
    }
}

impl JobDetails {
    #[must_use]
    pub fn new(job: &Job, schedule: Option<Cron>, command: Option<String>) -> Self {
        Self {
            job_id: job.guid(),
            job: job.clone(),
            schedule,
            next_tick: None,
            source: String::new(),
            command,
        }
    }
}
