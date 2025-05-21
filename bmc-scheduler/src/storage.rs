// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{JobDetails, jobs::JobId};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Storage trait for the job scheduler
#[async_trait::async_trait]
pub trait JobStorage: Serialize + Deserialize<'static> + Send + Sync + 'static {
    /// Submit a new job for immediate execution
    async fn save_job(&mut self, job: JobDetails) -> Result<()>;

    /// Get a job by ID
    async fn get_job(&self, job_id: &JobId) -> Result<Option<JobDetails>>;

    /// Load all jobs
    async fn load(&self) -> Result<Vec<JobDetails>>;

    /// Delete a job by ID
    async fn delete_job(&self, job_id: &JobId) -> Result<()>;
}
