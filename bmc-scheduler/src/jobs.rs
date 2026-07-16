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
