// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::JobDetails;
use anyhow::{anyhow, bail};
use chrono::{NaiveTime, Timelike};
pub use croner::Cron;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio_stream::{Stream, StreamExt};
use tracing::warn;

const PREFIX_SOURCE: &str = "###";
const PREFIX_COMMENT: &str = "#";
const CRON_DEFAULT_PATH: &str = "/etc/crontabs/root";
const CRON_DEFAULT_DIR: &str = "/etc/crontabs";
pub(crate) const CRON_DUMMY_COMMAND: &str = "true";
pub(crate) const CRON_SECONDS_PREFIX: &str = "0 ";
const MIN_CRON_FIELDS: usize = 6;

// Represents a single cron job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    /// The source system above the record, usually should be a first line (e.g., "### <Source>")
    pub source: Option<String>,
    /// The parsed schedule for evaluation (using Croner)
    pub schedule: Cron,
    /// The command to execute (e.g., "/path/to/script")
    pub command: String,
}

impl From<JobDetails> for CronEntry {
    fn from(job_details: JobDetails) -> Self {
        let schedule = job_details
            .schedule
            .unwrap_or(Cron::from_str("0 * * * * *").expect("BUG: Invalid cron expression"));
        let command = job_details.command.unwrap_or(CRON_DUMMY_COMMAND.to_owned());
        Self {
            source: Some(job_details.source),
            command,
            schedule,
        }
    }
}

impl CronEntry {
    /// Parse any number of lines
    fn from_lines(lines: Vec<String>) -> anyhow::Result<Vec<Self>> {
        let mut source = String::new();
        let mut cron_jobs = Vec::new();
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with(PREFIX_SOURCE) {
                line.clone_into(&mut source);
            } else if line.starts_with(PREFIX_COMMENT) {
                // Ignore comments
                continue;
            } else {
                let cron_job = parse_cron_block(&source, line.to_owned().as_str())?;
                cron_jobs.push(cron_job);
                source.clear();
            }
        }

        Ok(cron_jobs)
    }

    fn to_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(source) = &self.source {
            lines.push(format!("{PREFIX_SOURCE} {source}"));
        }

        // Add the cron line (schedule + command)
        let schedule_str = self.schedule.pattern.to_string();
        lines.push(format!("{} {}", schedule_str, self.command));

        lines
    }

    /// Serialize a vector of CronJob back to a crontab-formatted string
    #[must_use]
    pub fn to_crontab_string(&self) -> String {
        self.to_lines().join("\n")
    }
}

// Helper function to check if a string looks like a cron field (not a command path)
fn is_cron_field(field: &str) -> bool {
    // Cron fields contain only: numbers, wildcards, commas, hyphens, slashes, and specific keywords
    field
        .chars()
        .all(|c| c.is_ascii_digit() || c == '*' || c == ',' || c == '-' || c == '/')
        || matches!(
            field.to_uppercase().as_str(),
            "SUN"
                | "MON"
                | "TUE"
                | "WED"
                | "THU"
                | "FRI"
                | "SAT"
                | "JAN"
                | "FEB"
                | "MAR"
                | "APR"
                | "MAY"
                | "JUN"
                | "JUL"
                | "AUG"
                | "SEP"
                | "OCT"
                | "NOV"
                | "DEC"
        )
}

// Used to parse a single cron line and any optional comments
fn parse_cron_block(source: &str, cron_line: &str) -> anyhow::Result<CronEntry> {
    let source_parts = source.splitn(2, ' ').collect::<Vec<&str>>();

    let parts: Vec<&str> = cron_line.split_whitespace().collect();
    if parts.len() < MIN_CRON_FIELDS {
        bail!("Invalid cron line: too few fields: {cron_line}");
    }

    // Find where the command starts by looking for the first part that doesn't look like a cron field
    let mut command_start_idx = 5; // Default to 5-field format (min + hour + day + month + dayofweek)

    // Check if we have a 6-field format (with seconds)
    // Look at the 6th field (index 5) - if it looks like a cron field, we have 6-field format
    if parts.len() >= MIN_CRON_FIELDS && is_cron_field(parts[5]) {
        command_start_idx = MIN_CRON_FIELDS; // 6-field format (seconds + min + hour + day + month + dayofweek)
    }

    let expr = parts[0..command_start_idx].join(" ");
    let command = parts[command_start_idx..].join(" ");

    let source = if let Some(source) = source_parts.get(1) {
        (*source).to_owned().into()
    } else {
        None
    };

    // Try to parse the expression, with fallback to secondless version
    let schedule = match Cron::from_str(&expr) {
        Ok(cron) => cron,
        Err(_) => {
            // If parsing fails, try without seconds
            Cron::from_str(&expr)
                .map_err(|e| anyhow::anyhow!("Failed to parse cron expression '{}': {}", expr, e))?
        }
    };

    Ok(CronEntry {
        source,
        schedule,
        command,
    })
}

pub(crate) fn normalize_cron_expression(cron: Cron) -> anyhow::Result<Cron> {
    let cron_string = cron.pattern.to_string();
    let parts: Vec<&str> = cron_string.split(' ').collect();

    if parts.len() < MIN_CRON_FIELDS {
        let normalized = format!("{CRON_SECONDS_PREFIX}{cron_string}");
        Cron::from_str(&normalized).map_err(|e| anyhow!("Invalid cron expression: {e}"))
    } else {
        Ok(cron)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crontab {
    pub entries: Vec<CronEntry>,
    pub path: PathBuf,
}

impl Default for Crontab {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            path: PathBuf::from_str(CRON_DEFAULT_PATH)
                .expect("BUG: Failed to parse default crontab path"),
        }
    }
}

impl Crontab {
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or(
            PathBuf::from_str(CRON_DEFAULT_PATH)
                .expect("BUG: Failed to parse default crontab path"),
        );
        Self {
            path,
            ..Default::default()
        }
    }

    /// Load crontabs from all files in the /etc/crontabs/ directory
    pub async fn load_from_directory(&mut self) -> anyhow::Result<()> {
        let crontabs_dir = PathBuf::from(CRON_DEFAULT_DIR);

        // Ensure directory exists
        if !crontabs_dir.exists() {
            tokio::fs::create_dir_all(&crontabs_dir).await?;
            self.entries = Vec::new();
            return Ok(());
        }

        let mut all_entries = Vec::new();
        let mut dir_entries = tokio::fs::read_dir(&crontabs_dir).await?;

        while let Some(entry) = dir_entries.next_entry().await? {
            let entry_path = entry.path();

            // Skip if it's not a regular file
            if !entry_path.is_file() {
                continue;
            }

            // Skip hidden files and backup files
            if let Some(filename) = entry_path.file_name() {
                let filename_str = filename.to_string_lossy();
                if filename_str.starts_with('.') || filename_str.ends_with('~') {
                    continue;
                }
            }

            // Try to read and parse the file
            match tokio::fs::File::open(&entry_path).await {
                Ok(file) => match Self::read_full_crontab(file).await {
                    Ok(mut entries) => {
                        all_entries.append(&mut entries);
                    }
                    Err(e) => {
                        warn!("Failed to parse crontab file {:?}: {}", entry_path, e);
                    }
                },
                Err(e) => {
                    warn!("Failed to open crontab file {:?}: {}", entry_path, e);
                }
            }
        }

        self.entries = all_entries;
        Ok(())
    }

    /// Load crontab entries from disk, preserving all entries
    /// Always loads from the entire /etc/crontabs/ directory AND the configured path (if different)
    /// The configured path (self.path) only affects where new entries are saved
    pub async fn load_from_path(&mut self) -> anyhow::Result<()> {
        let mut all_entries = Vec::new();

        // Always load from the standard /etc/crontabs/ directory
        let mut temp_crontab = Crontab::default(); // Uses /etc/crontabs/ by default
        if let Ok(()) = temp_crontab.load_from_directory().await {
            all_entries.append(&mut temp_crontab.entries);
        }

        // If configured path is different from default, also load from it
        let default_path = PathBuf::from(CRON_DEFAULT_PATH);
        if self.path != default_path && !self.path.to_string_lossy().starts_with(CRON_DEFAULT_DIR) {
            if self.path.exists() {
                // Load from the custom path as well
                let file = tokio::fs::File::open(&self.path).await?;
                if let Ok(mut entries) = Self::read_full_crontab(file).await {
                    all_entries.append(&mut entries);
                }
            } else {
                // Create directories
                let parent_dir = self.path.parent().expect("BUG: Failed to parse parent dir");
                tokio::fs::create_dir_all(parent_dir).await?;
                // If file doesn't exist, create it empty
                tokio::fs::write(&self.path, "").await?;
            }
        }

        self.entries = all_entries;
        Ok(())
    }

    /// Add a new cron entry by appending to the root file
    /// Note: Loading reads from all files in /etc/crontabs/, but saving always goes to the root file only
    /// or to the specified path when Scheduler/Cron is initialized
    pub async fn add_entry(&mut self, entry: CronEntry) -> anyhow::Result<()> {
        // First load current state to keep our in-memory representation in sync
        self.load_from_path().await?;

        // Add to our in-memory collection
        self.entries.push(entry.clone());

        // Append to file
        let entry_lines = entry.to_lines();
        let mut entry_string = entry_lines.join("\n");
        entry_string.push('\n');

        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?
            .write_all(entry_string.as_bytes())
            .await?;

        Ok(())
    }

    /// Remove entries matching the given predicate
    pub async fn remove_entries<F>(&mut self, predicate: F) -> anyhow::Result<usize>
    where
        F: Fn(&CronEntry) -> bool,
    {
        // Load current state from disk
        self.load_from_path().await?;

        let original_count = self.entries.len();

        // Filter out entries that match the predicate
        self.entries.retain(|entry| !predicate(entry));

        let removed_count = original_count - self.entries.len();

        // If any entries were removed, rewrite the entire crontab file
        if removed_count > 0 {
            let mut crontab_content = self
                .entries
                .iter()
                .map(CronEntry::to_crontab_string)
                .collect::<Vec<String>>()
                .join("\n\n");
            
            // Ensure the file ends with a newline
            if !crontab_content.ends_with('\n') {
                crontab_content.push('\n');
            }
            
            tokio::fs::write(&self.path, crontab_content).await?;
        }

        Ok(removed_count)
    }

    /// Remove entries by source name
    pub async fn remove_by_source(&mut self, source: &str) -> anyhow::Result<usize> {
        self.remove_entries(|entry| entry.source.as_ref().is_some_and(|s| s == source))
            .await
    }

    /// Remove entries by command pattern
    pub async fn remove_by_command(&mut self, command: &str) -> anyhow::Result<usize> {
        self.remove_entries(|entry| entry.command == command).await
    }

    /// Remove entries by command containing a substring
    pub async fn remove_by_command_contains(&mut self, substring: &str) -> anyhow::Result<usize> {
        self.remove_entries(|entry| entry.command.contains(substring))
            .await
    }

    /// Check if an entry with the same source and command already exists
    pub async fn entry_exists(
        &mut self,
        source: Option<&str>,
        command: &str,
    ) -> anyhow::Result<bool> {
        self.load_from_path().await?;

        Ok(self
            .entries
            .iter()
            .any(|entry| entry.source.as_deref() == source && entry.command == command))
    }

    /// Replace existing entries with the same source, or add if not found
    pub async fn upsert_by_source(&mut self, entry: CronEntry) -> anyhow::Result<bool> {
        let source = entry.source.clone();
        let was_replaced = self
            .remove_by_source(source.as_deref().unwrap_or(""))
            .await?
            > 0;

        self.add_entry(entry).await?;
        Ok(was_replaced)
    }

    // Keep the original save method for backward compatibility, but add a warning
    pub async fn save(&self) -> anyhow::Result<()> {
        warn!(
            "save() overwrites entire crontab - consider using add_entry() or remove_entries() instead"
        );
        let mut crontab_content = self
            .entries
            .iter()
            .map(CronEntry::to_crontab_string)
            .collect::<Vec<String>>()
            .join("\n\n");
        
        // Ensure the file ends with a newline
        if !crontab_content.ends_with('\n') {
            crontab_content.push('\n');
        }
        
        tokio::fs::write(&self.path, crontab_content).await?;
        Ok(())
    }

    /// Returns Vec of Cron entries which don't run the default dummy command
    #[must_use]
    pub fn get_commands(&self) -> Vec<&CronEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.command.as_str() != CRON_DUMMY_COMMAND)
            .collect()
    }

    pub async fn read_full_crontab(
        stream: impl AsyncRead + Unpin,
    ) -> anyhow::Result<Vec<CronEntry>> {
        // Create a stream that reads lines from the async reader
        let lines_stream = Self::read_from_stream(stream);

        // Collect all lines from the stream into a Vec<String>
        let lines = lines_stream.collect::<Vec<String>>().await;

        // Parse the lines into CronEntry and create Crontab
        CronEntry::from_lines(lines)
    }

    pub fn read_from_stream(stream: impl AsyncRead + Unpin) -> impl Stream<Item = String> {
        let buf_reader = BufReader::new(stream);
        let lines_stream = tokio_stream::wrappers::LinesStream::new(buf_reader.lines());

        lines_stream.filter_map(std::result::Result::ok)
    }
}

pub fn from_naive_time(time: NaiveTime) -> anyhow::Result<Cron> {
    let hour = time.hour();
    let minute = time.minute();
    let second = time.second();

    Ok(Cron::from_str(&format!("{second} {minute} {hour} * * *"))?)
}

mod tests {
    #[test]
    fn test_cron_job_parse() {
        let crontab = r"### Source
            * * * * * /path/to/script
            # Another comment and indented line with seconds specified
                5 * * * * * /path/to/script2
            # Third job
            # with multiple lines
            # comment and no source
            * * * * * * /path/to/script3
            * * * * * * /path/to/script4
            * * * * * /path/to/script5 --arg1 --arg2";

        let cron_jobs =
            crate::cron::CronEntry::from_lines(crontab.lines().map(ToOwned::to_owned).collect())
                .expect("BUG: Failed to parse crontab");
        assert_eq!(cron_jobs.len(), 5);
        assert_eq!(
            cron_jobs[0]
                .source
                .clone()
                .expect("BUG: Wrongly parsed cron")
                .as_str(),
            "Source"
        );
        assert_eq!(
            cron_jobs[1].schedule.clone().pattern.to_string().as_str(),
            "5 * * * * *"
        );
        assert!(cron_jobs[1].source.clone().is_none());
        assert_eq!(
            cron_jobs[1].schedule.clone().pattern.to_string().as_str(),
            "5 * * * * *"
        );
    }
}
