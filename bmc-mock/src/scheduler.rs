// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_scheduler::{JobDetails, JobId, JobStorage};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct MockJobStorage;

#[async_trait::async_trait]
impl JobStorage for MockJobStorage {
    async fn save_job(&mut self, _job: JobDetails) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn get_job(&self, _job_id: &JobId) -> Result<Option<JobDetails>, anyhow::Error> {
        Ok(None)
    }

    async fn list_jobs(&self) -> Result<Vec<JobDetails>, anyhow::Error> {
        Ok(vec![])
    }

    async fn delete_job(&self, _job_id: &JobId) -> Result<(), anyhow::Error> {
        Ok(())
    }
}