// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::Cron;
use chrono::{DateTime, Utc};
pub use tokio_cron_scheduler::{Context as JobContext, Job, job::JobBuilder, job::JobId};

#[derive(Clone)]
pub struct JobDetails {
    pub job_id: JobId,
    pub job: Job,
    pub schedule: Option<Cron>,
    pub next_tick: Option<DateTime<Utc>>,
    /// Source of the job, e.g. "display", "upgrade", "notification"
    pub source: String,
}

impl std::fmt::Debug for JobDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "JobDetails {{ id: {:?}, schedule: {:?}, next_tick: {:?} }}",
            self.job_id, self.schedule, self.next_tick
        )
    }
}

impl JobDetails {
    #[must_use]
    pub fn new(job: &Job, schedule: Option<Cron>) -> Self {
        Self {
            job_id: job.guid(),
            job: job.clone(),
            schedule,   
            next_tick: None,
            source: String::new(),
        }
    }
}
