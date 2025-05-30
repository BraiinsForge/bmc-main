// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_scheduler::{JobDetails, JobId, JobStorage};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone)]
pub struct OpenwrtJobScheduler;

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct OpenwrtJobStorage;

#[async_trait::async_trait]
impl JobStorage for OpenwrtJobStorage {
    async fn save_job(&mut self, _job: JobDetails) -> Result<(), anyhow::Error> {
        todo!()
    }

    async fn get_job(&self, _job_id: &JobId) -> Result<Option<JobDetails>, anyhow::Error> {
        todo!()
    }

    async fn list_jobs(&self) -> Result<Vec<JobDetails>, anyhow::Error> {
        todo!()
    }

    async fn delete_job(&self, _job_id: &JobId) -> Result<(), anyhow::Error> {
        todo!()
    }
}