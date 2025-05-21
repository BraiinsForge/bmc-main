// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::Cron;
use chrono::{DateTime, Utc};
pub use tokio_cron_scheduler::{Context as JobContext, Job, job::JobBuilder, job::JobId};

#[derive(Clone)]
pub struct JobDetails {
    pub inner: Job,
    pub schedule: Option<Cron>,
    pub next_tick: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for JobDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "JobDetails {{ id: {:?}, next_tick: {:?} }}",
            self.inner.guid(),
            self.next_tick
        )
    }
}

impl JobDetails {
    #[must_use]
    pub fn new(job: Job, schedule: Option<Cron>) -> Self {
        Self {
            inner: job,
            schedule,
            next_tick: None,
        }
    }
}
