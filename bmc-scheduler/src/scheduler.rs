// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared::time::Timezone;
use chrono::{DateTime, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use croner::Cron;
use std::{collections::BTreeMap, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};
use tokio_cron_scheduler::Job;
pub use tokio_cron_scheduler::{JobScheduler as JobSchedulerLocked, JobSchedulerError};
use tracing::{error, info};

use crate::{JobDetails, jobs::JobId};

/// Main job scheduler service using tokio-cron-scheduler
#[derive(Clone)]
pub struct JobScheduler {
    inner: Arc<Mutex<JobSchedulerLocked>>,
    storage: Arc<RwLock<BTreeMap<JobId, JobDetails>>>,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
}

impl std::fmt::Debug for JobScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "JobScheduler {{ timezone_receiver: {:?} }}",
            self.timezone_receiver
        )
    }
}

impl JobScheduler {
    #[must_use]
    pub fn new(
        job_scheduler: JobSchedulerLocked,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    ) -> Self {
        let scheduler = Arc::new(Mutex::new(job_scheduler));
        Self {
            inner: scheduler,
            storage: Arc::new(RwLock::new(BTreeMap::new())),
            timezone_receiver,
        }
    }

    pub async fn init(&self) -> Result<(), JobSchedulerError> {
        let scheduler = self.inner.clone();
        info!("Starting scheduler");
        let inner = scheduler.lock().await;
        inner.start().await?;
        self.init_timezone_change();
        Ok(())
    }

    fn init_timezone_change(&self) {
        let timezone_receiver = self.timezone_receiver.clone();
        let scheduler_clone = self.inner.clone();
        let storage_clone = self.storage.clone();
        tokio::spawn(async move {
            info!("Starting timezone receiver");
            let mut timezone_receiver = timezone_receiver;
            loop {
                if timezone_receiver.changed().await.is_ok() {
                    let new_timezone: Timezone = timezone_receiver.borrow_and_update().clone();
                    info!("New timezone: {:?}", new_timezone);
                    let Ok(offset) = new_timezone.current_timezone_tz_offset() else {
                        error!("Error getting timezone offset: {:?}", new_timezone);
                        continue;
                    };
                    let jobs = list_jobs(storage_clone.clone(), scheduler_clone.clone())
                        .await
                        .map_err(|_| JobSchedulerError::GetJobData)
                        .unwrap_or_else(|_| vec![]);
                    info!("Updating jobs {}", jobs.len());
                    let date: DateTime<Tz> = DateTime::from_naive_utc_and_offset(
                        NaiveDateTime::from_timestamp(0, 0),
                        offset,
                    );
                    let date_timezone = date.timezone();
                    for mut job in jobs {
                        info!("guid: {:?}", job.job.guid());

                        let Ok(mut job_data) = job.job.job_data() else {
                            error!("Error getting job data: {:?}", job.job_id);
                            continue;
                        };

                        info!("job_data before update: {:?}", job_data);
                        job_data.timezone = date_timezone;
                        info!("job_data after update: {:?}", job_data);

                        if let Err(e) = job.job.set_job_data(job_data) {
                            error!("Error setting job data: {:?}", e);
                            continue;
                        }
                    }
                }
            }
        });
    }

    pub async fn submit_job_simple<T, Z>(&self, schedule: Cron, timezone: Z, source: String, callback: T) -> Result<JobId, JobSchedulerError>
    where
        T: 'static,
        T: FnMut(JobId, JobSchedulerLocked) -> Pin<Box<dyn Future<Output = ()> + Send>>
            + Send
            + Sync,
        Z: TimeZone + ToString,
    {
        if !schedule.pattern.with_seconds_required || !schedule.pattern.with_seconds_optional {
            error!("Seconds are required in Cron schedule");
            return Err(JobSchedulerError::ParseSchedule);
        }
        let job = Job::new_async_tz(&schedule.pattern, timezone, callback)?;
        let job_id = self.inner.lock().await.add(job.clone()).await?;
        let job_details = JobDetails {
            job_id,
            job,
            schedule: Some(schedule),
            next_tick: None,
            source,
        };
        self.storage.write().await.insert(job_id, job_details);
        Ok(job_id)
    }

    pub async fn submit_job_oneshot<T, Z>(&self, after: Duration, source: String, callback: T) -> Result<JobId, JobSchedulerError>
    where
        T: 'static,
        T: FnMut(JobId, JobSchedulerLocked) -> Pin<Box<dyn Future<Output = ()> + Send>>
            + Send
            + Sync,
        Z: TimeZone + ToString,
    {
        let job = Job::new_one_shot_async(after, callback)?;
        let job_id = self.inner.lock().await.add(job.clone()).await?;
        let job_details = JobDetails {
            job_id,
            job,
            source,
            schedule: None,
            next_tick: None,
        };
        self.storage.write().await.insert(job_id, job_details);
        Ok(job_id)
    }

    pub async fn get_job(&self, job_id: &JobId) -> Result<Option<JobDetails>, JobSchedulerError> {
        let map = self.storage.read().await;
        let Some(job_details) = map.get(job_id) else {
            return Ok(None);
        };
        let mut job_details = job_details.clone();
        let next_tick = self.inner.lock().await.next_tick_for_job(*job_id).await?;
        job_details.next_tick = next_tick;
        Ok(Some(job_details))
    }

    pub async fn time_till_next_job(&mut self) -> Result<Option<Duration>, JobSchedulerError> {
        self.inner.lock().await.time_till_next_job().await
    }

    pub async fn list_jobs(&self) -> Result<Vec<JobDetails>, JobSchedulerError> {
        list_jobs(self.storage.clone(), self.inner.clone()).await
    }

    pub async fn cancel_job(&self, job_id: &JobId) -> Result<(), JobSchedulerError> {
        self.inner.lock().await.remove(job_id).await
    }
}

async fn list_jobs(
    storage: Arc<RwLock<BTreeMap<JobId, JobDetails>>>,
    inner: Arc<Mutex<JobSchedulerLocked>>,
) -> Result<Vec<JobDetails>, JobSchedulerError> {
    let jobs = storage.read().await;
    let mut jobs_vec = Vec::new();
    let mut inner = inner.lock().await;

    for (job_id, job_details) in jobs.iter() {
        let mut job_details = job_details.clone();
        let next_tick = inner.next_tick_for_job(*job_id).await?;
        job_details.next_tick = next_tick;
        jobs_vec.push(job_details);
    }

    Ok(jobs_vec)
}
