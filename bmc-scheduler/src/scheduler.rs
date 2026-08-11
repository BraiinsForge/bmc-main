// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

use crate::JobDetails;
use crate::cron::{CRON_DUMMY_COMMAND, CronEntry, CrontabManager, normalize_cron_expression};
use anyhow::{Result, anyhow};
use bmc_shared_time::time::Timezone;
use chrono_tz::Tz;
use croner::Cron;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio_cron_scheduler::{Job, JobScheduler as JobSchedulerLocked, JobSchedulerError};
use tracing::{debug, error, warn};
use uuid::Uuid;

pub type AsyncTask = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
pub type JobId = Uuid;

#[derive(Debug, Clone)]
pub struct JobConfig {
    pub source: String,
    pub persist_to_crontab: bool,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            source: "unknown".to_owned(),
            persist_to_crontab: false,
        }
    }
}

impl JobConfig {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn persist(mut self) -> Self {
        self.persist_to_crontab = true;
        self
    }
}

#[expect(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Schedule {
    Cron(Cron),
    OneShot(Duration),
}

pub enum Task {
    Async(AsyncTask),
    Command(String),
}

impl Debug for Task {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Task::Async(_) => f.debug_tuple("Task::Async").finish(),
            Task::Command(_) => f.debug_tuple("Task::Command").finish(),
        }
    }
}

async fn list_jobs(
    storage: Arc<RwLock<BTreeMap<JobId, JobDetails>>>,
    inner: Arc<Mutex<JobSchedulerLocked>>,
) -> Result<Vec<JobDetails>> {
    let mut inner = inner.lock().await;
    let jobs = storage.read().await;
    let mut jobs_vec = Vec::new();

    for (job_id, job_details) in jobs.iter() {
        let mut job_details = job_details.clone();
        let next_tick = inner.next_tick_for_job(*job_id).await?;
        job_details.next_tick = next_tick;
        jobs_vec.push(job_details);
    }

    Ok(jobs_vec)
}

fn spawn_timezone_listener(
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    scheduler: Arc<Mutex<JobSchedulerLocked>>,
    storage: Arc<RwLock<BTreeMap<JobId, JobDetails>>>,
) {
    tokio::spawn(async move {
        debug!("Starting timezone receiver");
        let mut timezone_receiver = timezone_receiver;
        while timezone_receiver.changed().await.is_ok() {
            let new_timezone: Timezone = timezone_receiver.borrow_and_update().clone();
            debug!("New timezone: {:?}", new_timezone);
            let date_timezone = *new_timezone.chrono();
            let jobs = list_jobs(storage.clone(), scheduler.clone())
                .await
                .map_err(|_| JobSchedulerError::GetJobData)
                .unwrap_or_else(|_| vec![]);
            debug!("Updating jobs {}", jobs.len());

            let scheduler = scheduler.clone().lock_owned().await;
            let mut storage = storage.write().await;
            for mut job_details in jobs {
                let job_id = job_details.job_id;
                debug!("job_id before update: {:?}", job_id);
                if let Err(e) = scheduler.remove(&job_id).await {
                    error!("Error removing job: {:?}", e);
                    continue;
                }
                storage.remove(&job_id);

                let Ok(mut job_data) = job_details.job.job_data() else {
                    error!("Error getting job data: {:?}", job_details.job_id);
                    continue;
                };

                debug!("job_data before update: {:?}", job_data);
                job_data.set_timezone(date_timezone);
                debug!("job_data after update: {:?}", job_data);

                if let Err(e) = job_details.job.set_job_data(job_data) {
                    error!("Error setting job data: {:?}", e);
                    continue;
                }

                let Ok(job_id) = scheduler.add(job_details.job.clone()).await else {
                    error!("Error adding job: {:?}", job_details.job_id);
                    continue;
                };
                storage.insert(job_id, job_details);
                debug!("job_id after update: {:?}", job_id);
            }
        }
    });
}

/// Enhanced job scheduler wrapper around tokio-cron-scheduler
#[derive(Clone)]
pub struct JobScheduler {
    inner: Arc<Mutex<JobSchedulerLocked>>,
    storage: Arc<RwLock<BTreeMap<JobId, JobDetails>>>,
    crontab_manager: Arc<RwLock<CrontabManager>>,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
}

impl Debug for JobScheduler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobScheduler").finish()
    }
}

impl JobScheduler {
    pub async fn init(
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
        crontab_path: Option<PathBuf>,
    ) -> Self {
        let job_scheduler = JobSchedulerLocked::new()
            .await
            .expect("BUG: Failed to start scheduler");

        job_scheduler
            .start()
            .await
            .expect("BUG: Failed to start scheduler");

        let inner = Arc::new(Mutex::new(job_scheduler));
        let storage = Arc::new(RwLock::new(BTreeMap::new()));
        let mut crontab_manager = CrontabManager::new(crontab_path.clone());
        if crontab_path.is_some() {
            let _ = crontab_manager
                .load_all()
                .await
                .map_err(|e| warn!("Failed to load crontabs: {}", e));
            let _ = crontab_manager
                .ensure_scheduler_disclaimer()
                .await
                .map_err(|e| warn!("Failed to ensure scheduler crontab disclaimer: {}", e));
        }
        let crontab_manager = Arc::new(RwLock::new(crontab_manager));

        spawn_timezone_listener(timezone_receiver.clone(), inner.clone(), storage.clone());
        Self {
            inner,
            storage,
            crontab_manager,
            timezone_receiver,
        }
    }

    // MAIN API - Single unified scheduling method
    pub async fn schedule(
        &self,
        schedule: Schedule,
        task: Task,
        config: JobConfig,
    ) -> Result<Uuid> {
        let timezone = *self.timezone_receiver.borrow().chrono();
        let schedule = match schedule {
            Schedule::Cron(cron) => Schedule::Cron(normalize_cron_expression(cron)?),
            Schedule::OneShot(_) => schedule,
        };
        // Auto-persist to crontab if requested and it's a cron job with command
        if config.persist_to_crontab
            && let Schedule::Cron(cron) = &schedule
        {
            let command = match &task {
                Task::Command(cmd) => cmd,
                Task::Async(_) => CRON_DUMMY_COMMAND,
            };
            self.save_to_crontab(cron.clone(), command, &config)
                .await
                .map_err(|e| anyhow!("Failed to save to crontab: {e}"))?;
        }

        let job_id = self
            .internal_schedule(schedule.clone(), task, &config, timezone)
            .await
            .map_err(|e| anyhow!("Failed to schedule task: {e}"))?;

        Ok(job_id)
    }

    /// Timezone this scheduler evaluates cron patterns in.
    ///
    /// Callers deriving a pattern from a wall-clock instant must read that
    /// instant in this zone, not the system-local one.
    #[must_use]
    pub fn timezone(&self) -> Tz {
        *self.timezone_receiver.borrow().chrono()
    }

    // CONVENIENCE METHODS - Simple cases
    pub async fn schedule_cron(&self, cron: Cron, task: Task) -> Result<Uuid> {
        self.schedule(Schedule::Cron(cron), task, JobConfig::default())
            .await
    }

    pub async fn schedule_after(&self, duration: Duration, task: Task) -> Result<Uuid> {
        self.schedule(Schedule::OneShot(duration), task, JobConfig::default())
            .await
    }

    // BUILDER PATTERN ENTRY POINT
    #[must_use]
    pub fn new_job() -> JobBuilder {
        JobBuilder::new()
    }

    // SIMPLIFIED JOB MANAGEMENT
    pub async fn cancel(&self, job_id: &Uuid) -> Result<()> {
        self.inner.lock().await.remove(job_id).await?;
        let job_details = self.storage.write().await.remove(job_id);

        // Remove from crontab if it was persisted
        if let Some(details) = job_details {
            self.remove_from_crontab(details.source, details.command)
                .await?;
        }

        Ok(())
    }

    pub async fn cron_entries(&self) -> Vec<CronEntry> {
        let crontab_manager = self.crontab_manager.read().await;
        crontab_manager
            .get_all_entries()
            .into_iter()
            .cloned()
            .collect()
    }

    pub async fn jobs(&self) -> Result<Vec<JobDetails>> {
        list_jobs(self.storage.clone(), self.inner.clone()).await
    }

    pub async fn jobs_by_source(&self, source: &str) -> Result<Vec<JobDetails>> {
        let mut inner = self.inner.lock().await;
        let jobs = self.storage.read().await;
        let mut jobs_vec = Vec::new();

        for (job_id, job_details) in jobs.iter().filter(|job| job.1.source == source) {
            let mut job_details = job_details.clone();
            let next_tick = inner.next_tick_for_job(*job_id).await?;
            job_details.next_tick = next_tick;
            jobs_vec.push(job_details);
        }

        Ok(jobs_vec)
    }

    pub async fn job(&self, job_id: &Uuid) -> Result<Option<JobDetails>> {
        // This is here to keep the lock requirements the same as the rest of the methods
        let mut inner = self.inner.lock().await;
        let job_details = { self.storage.read().await.get(job_id).cloned() };

        let Some(mut job_details) = job_details else {
            return Ok(None);
        };

        let next_tick = inner.next_tick_for_job(*job_id).await?;
        job_details.next_tick = next_tick;

        Ok(Some(job_details))
    }

    pub async fn cancel_jobs(&self, source: String) {
        let job_ids: Vec<Uuid> = {
            self.storage
                .read()
                .await
                .iter()
                .filter(|(_job_id, job_details)| job_details.source == source)
                .map(|(job_id, _)| *job_id)
                .collect()
        };

        for job_id in &job_ids {
            if let Err(e) = self.cancel(job_id).await {
                warn!("Failed to cancel job: {}", e);
            } else {
                debug!(
                    "JobId: {}, source: {} was successfully canceled",
                    &job_id, &source
                );
            }
        }
    }

    pub async fn time_till_next_job(&mut self) -> Result<Option<Duration>, JobSchedulerError> {
        self.inner.lock().await.time_till_next_job().await
    }

    // CRONTAB MANAGEMENT
    pub async fn sync_with_crontab(&self) -> Result<Vec<Uuid>> {
        let crontab_manager = self.crontab_manager.read().await;
        let entries = crontab_manager.get_all_command_entries();
        let mut job_ids = Vec::new();

        for entry in entries {
            if entry.command == CRON_DUMMY_COMMAND {
                continue; // Skip Async tasks, those have to be added manually
            }
            let config =
                JobConfig::new(entry.source.clone().unwrap_or_else(|| "crontab".to_owned()));
            let job_id = self
                .schedule(
                    Schedule::Cron(entry.schedule.clone()),
                    Task::Command(entry.command.clone()),
                    config,
                )
                .await?;
            job_ids.push(job_id);
        }

        Ok(job_ids)
    }

    // INTERNAL METHODS
    async fn internal_schedule(
        &self,
        schedule: Schedule,
        task: Task,
        config: &JobConfig,
        timezone: Tz,
    ) -> Result<Uuid> {
        let (schedule_info, cmd, job) = match schedule {
            Schedule::Cron(cron) => {
                let (cmd, job) = Self::create_cron_job(&cron, task, timezone)?;
                (Some(cron), cmd, job)
            }
            Schedule::OneShot(duration) => {
                let (cmd, job) = Self::create_oneshot_job(duration, task)?;
                (None, cmd, job)
            }
        };

        self.add_job(job, schedule_info, config.source.clone(), cmd)
            .await
    }

    fn create_cron_job(cron: &Cron, task: Task, timezone: Tz) -> Result<(Option<String>, Job)> {
        match task {
            Task::Async(async_task) => {
                let wrapped_task =
                    move |_job_id: Uuid, _scheduler: JobSchedulerLocked| async_task();
                let job = Job::new_async_tz(cron, timezone, wrapped_task)
                    .map_err(|e| anyhow!("Cron+Async fail {e:?}"))?;
                Ok((None, job))
            }
            Task::Command(cmd) => {
                let cmd_clone = cmd.clone();
                let wrapped_task = Self::create_command_wrapper(cmd);
                let job = Job::new_async_tz(cron, timezone, wrapped_task)
                    .map_err(|e| anyhow!("Cron+Command fail {e:?}"))?;
                Ok((Some(cmd_clone), job))
            }
        }
    }

    fn create_oneshot_job(duration: Duration, task: Task) -> Result<(Option<String>, Job)> {
        match task {
            Task::Async(async_task) => {
                let wrapped_task =
                    move |_job_id: Uuid, _scheduler: JobSchedulerLocked| async_task();
                let job = Job::new_one_shot_async(duration, wrapped_task)
                    .map_err(|e| anyhow!("OneShot+Async fail {e:?}"))?;
                Ok((None, job))
            }
            Task::Command(cmd) => {
                let cmd_clone = cmd.clone();
                let wrapped_task = Self::create_command_wrapper(cmd);
                let job = Job::new_one_shot_async(duration, wrapped_task)
                    .map_err(|e| anyhow!("OneShot+Command fail {e:?}"))?;
                Ok((Some(cmd_clone), job))
            }
        }
    }

    fn create_command_wrapper(
        cmd: String,
    ) -> impl Fn(Uuid, JobSchedulerLocked) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        move |_job_id: Uuid, _scheduler: JobSchedulerLocked| {
            let cmd = cmd.clone();
            Box::pin(async move {
                if let Err(e) = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn() {
                    error!("Failed to spawn shell command '{}': {}", cmd, e);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        }
    }

    async fn save_to_crontab(&self, cron: Cron, command: &str, config: &JobConfig) -> Result<()> {
        let mut crontab_manager = self.crontab_manager.write().await;
        let entry = CronEntry {
            schedule: cron,
            command: command.to_owned(),
            source: Some(config.source.clone()),
        };
        crontab_manager
            .scheduler_crontab_mut()
            .upsert_by_source(entry)
            .await?;
        // Ensure disclaimer is maintained after saving
        crontab_manager
            .ensure_scheduler_disclaimer()
            .await
            .map_err(|e| anyhow!("Failed to ensure disclaimer: {e}"))?;
        Ok(())
    }

    async fn remove_from_crontab(&self, source: String, command: Option<String>) -> Result<usize> {
        let mut crontab_manager = self.crontab_manager.write().await;
        let scheduler_crontab = crontab_manager.scheduler_crontab_mut();
        let result = if let Some(command) = command {
            scheduler_crontab.remove_by_command(&command).await
        } else if !source.is_empty() {
            scheduler_crontab.remove_by_source(&source).await
        } else {
            warn!("No source or command specified to remove from crontab");
            Ok(0)
        };

        // Ensure disclaimer is maintained after removal
        if let Ok(count) = &result
            && count > &0
        {
            let _ = crontab_manager
                .ensure_scheduler_disclaimer()
                .await
                .map_err(|e| warn!("Failed to ensure disclaimer after removal: {}", e));
        }

        result
    }

    async fn add_job(
        &self,
        job: Job,
        schedule: Option<Cron>,
        source: String,
        command: Option<String>,
    ) -> Result<Uuid> {
        let mut inner = self.inner.lock().await;
        let job_id = inner.add(job.clone()).await?;
        let next_tick = inner.next_tick_for_job(job_id).await?;
        debug!(">>> Job from '{source}' added: job_id: {job_id}, next_tick: {next_tick:?}");
        let job_details = JobDetails {
            job_id,
            job,
            schedule,
            next_tick,
            source,
            command,
        };
        self.storage.write().await.insert(job_id, job_details);
        Ok(job_id)
    }
}

// BUILDER PATTERN
#[derive(Debug)]
pub struct JobBuilder {
    config: JobConfig,
    schedule: Option<Schedule>,
    task: Option<Task>,
}

impl JobBuilder {
    #[must_use]
    fn new() -> Self {
        Self {
            config: JobConfig::default(),
            schedule: None,
            task: None,
        }
    }

    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.config.source = source.into();
        self
    }

    #[must_use]
    pub fn persist(mut self) -> Self {
        self.config.persist_to_crontab = true;
        self
    }

    #[must_use]
    pub fn cron(mut self, cron: Cron) -> Self {
        self.schedule = Some(Schedule::Cron(cron));
        self
    }

    #[must_use]
    pub fn after(mut self, duration: Duration) -> Self {
        self.schedule = Some(Schedule::OneShot(duration));
        self
    }

    #[must_use]
    pub fn command(mut self, cmd: impl Into<String>) -> Self {
        self.task = Some(Task::Command(cmd.into()));
        self
    }

    #[must_use]
    pub fn async_task(mut self, task: AsyncTask) -> Self {
        self.task = Some(Task::Async(task));
        self
    }

    #[must_use]
    pub fn task<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let boxed = Box::new(move || {
            let fut = f();
            Box::pin(fut) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        self.task = Some(Task::Async(boxed));
        self
    }

    pub async fn schedule(self, scheduler: &JobScheduler) -> Result<Uuid> {
        let schedule = self
            .schedule
            .ok_or_else(|| anyhow::anyhow!("No schedule specified"))?;
        let task = self
            .task
            .ok_or_else(|| anyhow::anyhow!("No task specified"))?;

        scheduler.schedule(schedule, task, self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxedTask;
    use std::str::FromStr;
    use tempfile::TempDir;
    use tokio::time::Duration;
    // Helper to create a test scheduler
    async fn create_test_scheduler() -> (JobScheduler, TempDir) {
        let temp_dir = TempDir::new().expect("BUG: Failed to create temp dir");
        let crontab_path = Some(temp_dir.path().join("test_crontab"));
        let (_, rx) = tokio::sync::watch::channel(Timezone::default());
        let scheduler = JobScheduler::init(rx, crontab_path).await;
        (scheduler, temp_dir)
    }
    fn boxed_task() -> BoxedTask {
        Box::new(|| Box::pin(async {}) as Pin<Box<dyn Future<Output = ()> + Send>>)
    }
    #[tokio::test]
    async fn test_schedule_async_task_is_registered() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let task = boxed_task();
        let job_id = scheduler
            .schedule_after(Duration::from_millis(10), Task::Async(task))
            .await
            .expect("BUG: Failed to schedule task");
        // Verify job exists in storage immediately after scheduling
        let job_details = scheduler
            .job(&job_id)
            .await
            .expect("BUG: Failed to get job details");
        assert!(job_details.is_some());
        // Verify it's in the jobs list
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, job_id);
    }
    #[tokio::test]
    async fn test_valid_cron_expressions() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        // Test various valid cron expressions
        let valid_crons = [
            "0 0 * * *",     // Daily at midnight
            "0 0 */2 * * *", // Every 2 hours
            "30 9 * * 1-5",  // 9:30 AM on weekdays
            "0 0 1 * *",     // First day of every month
            "*/15 * * * *",  // Every 15 minutes
            "0 0 * * 0",     // Every Sunday
            "0 12 * * MON",  // Every Monday at noon
        ];
        for cron_expr in &valid_crons {
            let cron =
                Cron::from_str(cron_expr).expect("BUG: Failed to parse valid cron expression");
            let job_id = scheduler
                .schedule_cron(cron, Task::Async(boxed_task()))
                .await
                .map_err(|e| format!("{cron_expr}: Error: {e}"))
                .expect("BUG: Failed to schedule cron job");
            // Verify job is scheduled
            let job_details = scheduler
                .job(&job_id)
                .await
                .expect("BUG: Failed to get job details");
            assert!(job_details.is_some());
            assert_eq!(
                job_details.expect("BUG: Job details should exist").source,
                "unknown"
            );
        }
        // Verify all jobs are registered
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), valid_crons.len());
    }
    #[tokio::test]
    async fn test_invalid_cron_expressions() {
        // Test that invalid cron expressions fail at parse time, not at schedule time
        let invalid_crons = [
            "invalid cron",
            "60 0 * * *",      // Invalid minute (>59)
            "0 25 * * *",      // Invalid hour (>23)
            "0 0 32 * *",      // Invalid day (>31)
            "0 0 * 13 *",      // Invalid month (>12)
            "0 0 * * 8",       // Invalid weekday (>7)
            "",                // Empty string
            "* * * *",         // Too few fields
            "* * * * * * * *", // Too many fields
        ];
        for cron_expr in &invalid_crons {
            let result = Cron::from_str(cron_expr);
            assert!(
                result.is_err(),
                "Cron expression '{cron_expr}' should be invalid",
            );
        }
    }
    #[tokio::test]
    async fn test_job_config_with_source() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let config = JobConfig::new("test_source");
        let task = boxed_task();
        let job_id = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_millis(10)),
                Task::Async(task),
                config,
            )
            .await
            .expect("BUG: Failed to schedule job with config");
        let job_details = scheduler
            .job(&job_id)
            .await
            .expect("BUG: Failed to get job details")
            .expect("BUG: Job details should exist");
        assert_eq!(job_details.source, "test_source");
    }
    #[tokio::test]
    async fn test_cancel_job() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let task = boxed_task();
        let job_id = scheduler
            .schedule_after(Duration::from_mins(1), Task::Async(task))
            .await
            .expect("BUG: Failed to schedule job");
        // Verify job exists
        assert!(
            scheduler
                .job(&job_id)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
        // Cancel job
        scheduler
            .cancel(&job_id)
            .await
            .expect("BUG: Failed to cancel job");
        // Verify job is removed
        assert!(
            scheduler
                .job(&job_id)
                .await
                .expect("BUG: Failed to get job details")
                .is_none()
        );
    }
    #[tokio::test]
    async fn test_cancel_jobs_by_source() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let config1 = JobConfig::new("source1");
        let config2 = JobConfig::new("source2");
        let job_id1 = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_mins(1)),
                Task::Async(boxed_task()),
                config1.clone(),
            )
            .await
            .expect("BUG: Failed to schedule job 1");
        let job_id2 = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_mins(1)),
                Task::Async(boxed_task()),
                config1,
            )
            .await
            .expect("BUG: Failed to schedule job 2");
        let job_id3 = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_mins(1)),
                Task::Async(boxed_task()),
                config2,
            )
            .await
            .expect("BUG: Failed to schedule job 3");
        // Cancel all jobs from source1
        scheduler.cancel_jobs("source1".to_owned()).await;
        // Verify source1 jobs are removed
        assert!(
            scheduler
                .job(&job_id1)
                .await
                .expect("BUG: Failed to get job details")
                .is_none()
        );
        assert!(
            scheduler
                .job(&job_id2)
                .await
                .expect("BUG: Failed to get job details")
                .is_none()
        );
        // Verify source2 job still exists
        assert!(
            scheduler
                .job(&job_id3)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
    }
    #[tokio::test]
    async fn test_list_jobs() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let _job_id1 = scheduler
            .schedule_after(Duration::from_mins(1), Task::Async(boxed_task()))
            .await
            .expect("BUG: Failed to schedule job 1");
        let _job_id2 = scheduler
            .schedule_after(Duration::from_mins(1), Task::Async(boxed_task()))
            .await
            .expect("BUG: Failed to schedule job 2");
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), 2);
    }
    #[tokio::test]
    async fn test_builder_pattern_validation() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let job_id = JobScheduler::new_job()
            .source("test_builder")
            .after(Duration::from_millis(10))
            .task(|| async {})
            .schedule(&scheduler)
            .await
            .expect("BUG: Failed to schedule job with builder");
        let job_details = scheduler
            .job(&job_id)
            .await
            .expect("BUG: Failed to get job details")
            .expect("BUG: Job details should exist");
        assert_eq!(job_details.source, "test_builder");
    }
    #[tokio::test]
    async fn test_command_task_registration() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let job_id = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_millis(10)),
                Task::Command("echo test".to_owned()),
                JobConfig::new("command_test"),
            )
            .await
            .expect("BUG: Failed to schedule command task");
        // Job should be registered in storage
        let job_details = scheduler
            .job(&job_id)
            .await
            .expect("BUG: Failed to get job details");
        assert!(job_details.is_some());
        assert_eq!(
            job_details.expect("BUG: Job details should exist").source,
            "command_test"
        );
    }
    #[tokio::test]
    async fn test_crontab_persistence_logic() {
        let (scheduler, temp_dir) = create_test_scheduler().await;
        let cron = Cron::from_str("0 0 * * *").expect("BUG: Failed to parse cron expression"); // Daily at midnight
        let config = JobConfig::new("persistent_test").persist();
        let job_id = scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Command("echo daily".to_owned()),
                config,
            )
            .await
            .expect("BUG: Failed to schedule persistent job");
        // Check if crontab file was created/updated
        let crontab_path = temp_dir.path().join("test_crontab");
        assert!(crontab_path.exists());
        // Verify job is also in memory storage
        let job_details = scheduler
            .job(&job_id)
            .await
            .expect("BUG: Failed to get job details");
        assert!(job_details.is_some());
    }
    #[tokio::test]
    async fn test_schedule_types_mapping() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        // Test OneShot schedule
        let job_id1 = scheduler
            .schedule(
                Schedule::OneShot(Duration::from_millis(100)),
                Task::Async(boxed_task()),
                JobConfig::new("oneshot_test"),
            )
            .await
            .expect("BUG: Failed to schedule oneshot job");
        // Test Cron schedule
        let cron = Cron::from_str("0 0 * * *").expect("BUG: Failed to parse cron expression");
        let job_id2 = scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Async(boxed_task()),
                JobConfig::new("cron_test"),
            )
            .await
            .expect("BUG: Failed to schedule cron job");
        // Verify both are scheduled
        assert!(
            scheduler
                .job(&job_id1)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
        assert!(
            scheduler
                .job(&job_id2)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), 2);
    }
    #[tokio::test]
    async fn test_builder_missing_schedule() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let result = JobScheduler::new_job()
            .source("test")
            .task(|| async {})
            .schedule(&scheduler)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .expect_err("BUG: Result should be an error")
                .to_string()
                .contains("No schedule specified")
        );
    }
    #[tokio::test]
    async fn test_builder_missing_task() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        let result = JobScheduler::new_job()
            .source("test")
            .after(Duration::from_millis(10))
            .schedule(&scheduler)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .expect_err("BUG: Result should be an error")
                .to_string()
                .contains("No task specified")
        );
    }
    #[tokio::test]
    async fn test_job_config_default() {
        let config = JobConfig::default();
        assert_eq!(config.source, "unknown");
        assert!(!config.persist_to_crontab);
    }
    #[tokio::test]
    async fn test_job_config_builder() {
        let config = JobConfig::new("test_source").persist();
        assert_eq!(config.source, "test_source");
        assert!(config.persist_to_crontab);
    }
    #[tokio::test]
    async fn test_concurrent_job_scheduling() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        // Schedule multiple jobs concurrently
        let mut handles = vec![];
        for i in 0..10 {
            let scheduler_clone = scheduler.clone();
            let handle = tokio::spawn(async move {
                scheduler_clone
                    .schedule(
                        Schedule::OneShot(Duration::from_millis(10)),
                        Task::Async(boxed_task()),
                        JobConfig::new(format!("concurrent_test_{i}")),
                    )
                    .await
            });
            handles.push(handle);
        }
        // Wait for all scheduling to complete
        let mut job_ids = vec![];
        for handle in handles {
            let job_id = handle
                .await
                .expect("BUG: Failed to await handle")
                .expect("BUG: Failed to schedule concurrent job");
            job_ids.push(job_id);
        }
        assert_eq!(job_ids.len(), 10);
        // Verify all jobs are in storage
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), 10);
        // Verify each job has correct source
        for job in &jobs {
            assert!(job.source.starts_with("concurrent_test_"));
        }
    }
    #[tokio::test]
    async fn test_builder_with_different_schedule_types() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        // Test builder with cron schedule
        let job_id1 = JobScheduler::new_job()
            .source("cron_builder")
            .cron(Cron::from_str("0 0 * * *").expect("BUG: Failed to parse cron expression"))
            .task(|| async {})
            .schedule(&scheduler)
            .await
            .expect("BUG: Failed to schedule cron job with builder");
        // Test builder with after schedule
        let job_id2 = JobScheduler::new_job()
            .source("after_builder")
            .after(Duration::from_millis(100))
            .task(|| async {})
            .schedule(&scheduler)
            .await
            .expect("BUG: Failed to schedule after job with builder");
        // Both should be scheduled successfully
        assert!(
            scheduler
                .job(&job_id1)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
        assert!(
            scheduler
                .job(&job_id2)
                .await
                .expect("BUG: Failed to get job details")
                .is_some()
        );
    }
    #[tokio::test]
    async fn test_complex_cron_expressions() {
        let (scheduler, _temp_dir) = create_test_scheduler().await;
        // Test more complex but valid cron expressions
        let complex_crons = [
            ("0 0 1 1 *", "New Year's Day"),
            ("0 0 * * 1-5", "Weekdays only"),
            (
                "*/10 9-17 * * 1-5",
                "Every 10 minutes during business hours",
            ),
            ("0 */4 * * *", "Every 4 hours"),
            ("30 2 * * 0", "2:30 AM every Sunday"),
        ];
        for (cron_expr, _) in complex_crons {
            let cron =
                Cron::from_str(cron_expr).expect("BUG: Failed to parse complex cron expression");
            let config = JobConfig::new("complex_test");
            let job_id = scheduler
                .schedule(Schedule::Cron(cron), Task::Async(boxed_task()), config)
                .await
                .expect("BUG: Failed to schedule complex cron job");
            let job_details = scheduler
                .job(&job_id)
                .await
                .expect("BUG: Failed to get job details")
                .expect("BUG: Job details should exist");
            assert_eq!(
                job_details
                    .schedule
                    .expect("BUG: Wrong cron pattern")
                    .pattern
                    .to_string()
                    .as_str(),
                format!("0 {cron_expr}").as_str()
            );
        }
        // Verify all complex crons are scheduled
        let jobs = scheduler
            .jobs()
            .await
            .expect("BUG: Failed to get jobs list");
        assert_eq!(jobs.len(), complex_crons.len());
    }
}
