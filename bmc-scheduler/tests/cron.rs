// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_scheduler::cron::*;
use std::str::FromStr;
use tempfile::NamedTempFile;
use tokio::io::AsyncReadExt;

// Helper function to create a test CronEntry
fn create_test_entry(source: Option<&str>, schedule: &str, command: &str) -> CronEntry {
    CronEntry {
        source: source.map(ToOwned::to_owned),
        schedule: Cron::from_str(schedule).expect("BUG: Failed to parse schedule"),
        command: command.to_owned(),
    }
}

// Helper function to create a temporary crontab file with content
async fn create_temp_crontab_with_content(
    content: &str,
) -> anyhow::Result<(Crontab, NamedTempFile)> {
    let temp_file = NamedTempFile::new()?;
    tokio::fs::write(temp_file.path(), content).await?;
    let crontab = Crontab::new(Some(temp_file.path().to_path_buf()));
    Ok((crontab, temp_file))
}

// Helper function to read file content as string
async fn read_file_content(path: &std::path::Path) -> anyhow::Result<String> {
    let mut content = String::new();
    let mut file = tokio::fs::File::open(path).await?;
    file.read_to_string(&mut content).await?;
    Ok(content)
}

#[tokio::test]
async fn test_load_empty_file() -> anyhow::Result<()> {
    let temp_file = NamedTempFile::new()?;
    tokio::fs::write(temp_file.path(), "").await?;

    let mut crontab = Crontab::new(Some(temp_file.path().to_path_buf()));
    crontab.load_from_path().await?;

    assert_eq!(crontab.entries.len(), 0);
    Ok(())
}

#[tokio::test]
async fn test_load_nonexistent_file() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let nonexistent_path = temp_dir.path().join("nonexistent.cron");

    let mut crontab = Crontab::new(Some(nonexistent_path.clone()));
    crontab.load_from_path().await?;

    assert_eq!(crontab.entries.len(), 0);
    assert!(nonexistent_path.exists()); // Should create empty file
    Ok(())
}

#[tokio::test]
async fn test_load_existing_crontab() -> anyhow::Result<()> {
    let content = r"### MyApp
0 2 * * * /path/to/daily.sh
### AnotherApp
0 * * * * /path/to/hourly.sh";

    let (mut crontab, _temp_file) = create_temp_crontab_with_content(content).await?;
    crontab.load_from_path().await?;

    assert_eq!(crontab.entries.len(), 2);
    assert_eq!(crontab.entries[0].source, Some("MyApp".to_owned()));
    assert_eq!(crontab.entries[1].source, Some("AnotherApp".to_owned()));
    Ok(())
}

#[tokio::test]
async fn test_add_entry_to_empty_file() -> anyhow::Result<()> {
    let temp_file = NamedTempFile::new()?;
    tokio::fs::write(temp_file.path(), "").await?;

    let mut crontab = Crontab::new(Some(temp_file.path().to_path_buf()));
    let entry = create_test_entry(Some("TestApp"), "0 1 * * *", "/test/command");

    crontab.add_entry(entry).await?;

    assert_eq!(crontab.entries.len(), 1);

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(file_content.contains("### TestApp"));
    assert!(file_content.contains("0 1 * * * /test/command"));
    Ok(())
}

#[tokio::test]
async fn test_add_entry_to_existing_file() -> anyhow::Result<()> {
    let initial_content = r"### ExistingApp
0 2 * * * /existing/command";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(initial_content).await?;

    let new_entry = create_test_entry(Some("NewApp"), "0 3 * * *", "/new/command");

    crontab.add_entry(new_entry).await?;

    assert_eq!(crontab.entries.len(), 2);

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(file_content.contains("ExistingApp"));
    assert!(file_content.contains("NewApp"));
    assert!(file_content.contains("/existing/command"));
    assert!(file_content.contains("/new/command"));
    Ok(())
}

#[tokio::test]
async fn test_remove_entries_with_predicate() -> anyhow::Result<()> {
    let content = r"### App1
0 1 * * * /app1/command1
### App2
0 2 * * * /app2/command2
### App1
0 3 * * * /app1/command3";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    // Remove all entries from App1
    let removed_count = crontab
        .remove_entries(|entry| entry.source.as_ref().is_some_and(|s| s == "App1"))
        .await?;

    assert_eq!(removed_count, 2);
    assert_eq!(crontab.entries.len(), 1);
    assert_eq!(crontab.entries[0].source, Some("App2".to_owned()));

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(!file_content.contains("App1"));
    assert!(file_content.contains("App2"));
    Ok(())
}

#[tokio::test]
async fn test_remove_entries_none_match() -> anyhow::Result<()> {
    let content = r"### App1
0 1 * * * /app1/command";

    let (mut crontab, _temp_file) = create_temp_crontab_with_content(content).await?;

    let removed_count = crontab
        .remove_entries(|entry| entry.source.as_ref().is_some_and(|s| s == "NonExistentApp"))
        .await?;

    assert_eq!(removed_count, 0);
    assert_eq!(crontab.entries.len(), 1);
    Ok(())
}

#[tokio::test]
async fn test_remove_by_source() -> anyhow::Result<()> {
    let content = r"### MyApp
0 1 * * * /command1
### OtherApp
0 2 * * * /command2
### MyApp
0 3 * * * /command3";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    let removed_count = crontab.remove_by_source("MyApp").await?;

    assert_eq!(removed_count, 2);
    assert_eq!(crontab.entries.len(), 1);
    assert_eq!(crontab.entries[0].source, Some("OtherApp".to_owned()));

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(!file_content.contains("MyApp"));
    assert!(file_content.contains("OtherApp"));
    Ok(())
}

#[tokio::test]
async fn test_remove_by_source_not_found() -> anyhow::Result<()> {
    let content = r"### MyApp
0 1 * * * /command1";

    let (mut crontab, _temp_file) = create_temp_crontab_with_content(content).await?;

    let removed_count = crontab.remove_by_source("NonExistentApp").await?;

    assert_eq!(removed_count, 0);
    assert_eq!(crontab.entries.len(), 1);
    Ok(())
}

#[tokio::test]
async fn test_remove_by_command() -> anyhow::Result<()> {
    let content = r"### App1
0 1 * * * /path/to/script.sh
### App2
0 2 * * * /other/command
### App3
0 3 * * * /path/to/script.sh
0 3 * * * /path/to/script2.sh";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    let removed_count = crontab.remove_by_command("/path/to/script.sh").await?;

    assert_eq!(removed_count, 2);
    assert_eq!(crontab.entries.len(), 2);
    assert_eq!(crontab.entries[0].command, "/other/command");

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(!file_content.contains("/path/to/script.sh"));
    assert!(file_content.contains("/other/command"));
    Ok(())
}

#[tokio::test]
async fn test_remove_by_command_contains() -> anyhow::Result<()> {
    let content = r"### App1
0 1 * * * /path/to/backup.sh --daily
### App2
0 2 * * * /other/command
### App3
0 3 * * * /scripts/backup.sh --weekly
0 3 * * * /path/to/script2.sh";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    let removed_count = crontab.remove_by_command_contains("backup.sh").await?;

    assert_eq!(removed_count, 2);
    assert_eq!(crontab.entries.len(), 2);
    assert_eq!(crontab.entries[0].command, "/other/command");

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(!file_content.contains("backup.sh"));
    assert!(file_content.contains("/other/command"));
    Ok(())
}

#[tokio::test]
async fn test_entry_exists() -> anyhow::Result<()> {
    let content = r"### MyApp
0 1 * * * /my/command
### OtherApp
0 2 * * * /other/command";

    let (mut crontab, _temp_file) = create_temp_crontab_with_content(content).await?;

    // Test existing entry
    assert!(crontab.entry_exists(Some("MyApp"), "/my/command").await?);

    // Test non-existing source
    assert!(
        !crontab
            .entry_exists(Some("NonExistent"), "/my/command")
            .await?
    );

    // Test non-existing command
    assert!(
        !crontab
            .entry_exists(Some("MyApp"), "/nonexistent/command")
            .await?
    );

    // Test entry without source
    assert!(!crontab.entry_exists(None, "/my/command").await?);
    Ok(())
}

#[tokio::test]
async fn test_entry_exists_no_source() -> anyhow::Result<()> {
    let content = r"0 1 * * * /no/source/command";

    let (mut crontab, _temp_file) = create_temp_crontab_with_content(content).await?;

    assert!(crontab.entry_exists(None, "/no/source/command").await?);
    assert!(
        !crontab
            .entry_exists(Some("SomeSource"), "/no/source/command")
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn test_upsert_by_source_replace_existing() -> anyhow::Result<()> {
    let content = r"### MyApp
0 1 * * * /old/command
### OtherApp
0 2 * * * /other/command";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    let new_entry = create_test_entry(Some("MyApp"), "0 3 * * *", "/new/command");

    let was_replaced = crontab.upsert_by_source(new_entry).await?;

    assert!(was_replaced);
    assert_eq!(crontab.entries.len(), 2);

    // Find MyApp entry and verify it was updated
    let myapp_entry = crontab
        .entries
        .iter()
        .find(|e| e.source.as_ref().is_some_and(|s| s == "MyApp"))
        .expect("BUG: Failed to find MyApp entry");
    assert_eq!(myapp_entry.command, "/new/command");

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(!file_content.contains("/old/command"));
    assert!(file_content.contains("/new/command"));
    Ok(())
}

#[tokio::test]
async fn test_upsert_by_source_add_new() -> anyhow::Result<()> {
    let content = r"### ExistingApp
0 1 * * * /existing/command";

    let (mut crontab, temp_file) = create_temp_crontab_with_content(content).await?;

    let new_entry = create_test_entry(Some("NewApp"), "0 2 * * *", "/new/command");

    let was_replaced = crontab.upsert_by_source(new_entry).await?;

    assert!(!was_replaced);
    assert_eq!(crontab.entries.len(), 2);

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(file_content.contains("ExistingApp"));
    assert!(file_content.contains("NewApp"));
    assert!(file_content.contains("/existing/command"));
    assert!(file_content.contains("/new/command"));
    Ok(())
}

#[tokio::test]
async fn test_save_backward_compatibility() -> anyhow::Result<()> {
    let temp_file = NamedTempFile::new()?;

    let entry1 = create_test_entry(Some("App1"), "0 1 * * *", "/command1");
    let entry2 = create_test_entry(Some("App2"), "0 2 * * *", "/command2");

    let crontab = Crontab {
        entries: vec![entry1, entry2],
        path: temp_file.path().to_path_buf(),
    };

    crontab.save().await?;

    let file_content = read_file_content(temp_file.path()).await?;
    assert!(file_content.contains("### App1"));
    assert!(file_content.contains("### App2"));
    assert!(file_content.contains("/command1"));
    assert!(file_content.contains("/command2"));
    Ok(())
}

#[tokio::test]
async fn test_scheduler_disclaimer() {
    use std::str::FromStr;
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let crontab_path = temp_dir.path().join("test_scheduler_crontab");

    let mut crontab = Crontab::new(Some(crontab_path.clone()));

    // First, add an entry using the proper method (upsert_by_source)
    let test_entry = CronEntry {
        source: Some("test".to_owned()),
        schedule: Cron::from_str("0 0 * * *").expect("Valid cron"),
        command: "echo test".to_owned(),
    };
    crontab
        .upsert_by_source(test_entry)
        .await
        .expect("Should add entry");

    // Now ensure disclaimer is added
    crontab
        .ensure_disclaimer()
        .await
        .expect("Should add disclaimer");

    // Read the file and verify disclaimer is at the top
    let content = tokio::fs::read_to_string(&crontab_path)
        .await
        .expect("Should read file");

    // Verify disclaimer is present at the start
    assert!(
        content
            .trim_start()
            .starts_with(Crontab::SCHEDULER_CRONTAB_DISCLAIMER)
    );

    // Verify the entry is also present
    assert!(content.contains("### test"));
    assert!(content.contains("0 0 * * * echo test"));

    // Test that calling ensure_disclaimer again doesn't duplicate
    crontab
        .ensure_disclaimer()
        .await
        .expect("Should not fail on second call");
    let content2 = tokio::fs::read_to_string(&crontab_path)
        .await
        .expect("Should read file again");

    // Content should be identical (no duplicate disclaimer)
    assert_eq!(content, content2);

    // Count occurrences of disclaimer to ensure no duplication
    let disclaimer_count = content
        .matches(Crontab::SCHEDULER_CRONTAB_DISCLAIMER)
        .count();
    assert_eq!(disclaimer_count, 1, "Should have exactly one disclaimer");
}

#[tokio::test]
async fn test_disclaimer_with_empty_crontab() {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let crontab_path = temp_dir.path().join("test_empty_crontab");

    let mut crontab = Crontab::new(Some(crontab_path.clone()));

    // Ensure disclaimer on empty crontab
    crontab
        .ensure_disclaimer()
        .await
        .expect("Should add disclaimer to empty crontab");

    let content = tokio::fs::read_to_string(&crontab_path)
        .await
        .expect("Should read file");

    // Should only contain disclaimer and proper newlines
    let disclaimer = Crontab::SCHEDULER_CRONTAB_DISCLAIMER;
    let expected = format!("{disclaimer}\n");
    assert_eq!(content, expected);
}

#[tokio::test]
async fn test_crontab_manager_load_all() {
    use std::path::PathBuf;
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let crontabs_dir = temp_dir.path().join("crontabs");
    tokio::fs::create_dir_all(&crontabs_dir)
        .await
        .expect("Failed to create crontabs directory");

    // Create scheduler's crontab
    let scheduler_path = crontabs_dir.join("scheduler");
    let scheduler_content = r"### scheduler
0 0 12 * * * /usr/bin/scheduler-task
### scheduler-backup
0 30 2 * * * /usr/bin/backup-task";
    tokio::fs::write(&scheduler_path, scheduler_content)
        .await
        .expect("Failed to write scheduler crontab");

    // Create system crontab 1
    let system1_path = crontabs_dir.join("root");
    let system1_content = r"### system
0 0 6 * * * /usr/bin/system-task
0 15 18 * * * /usr/bin/daily-maintenance";
    tokio::fs::write(&system1_path, system1_content)
        .await
        .expect("Failed to write system1 crontab");

    // Create system crontab 2
    let system2_path = crontabs_dir.join("user");
    let system2_content = r"### user-tasks
0 0 0 * * 1 /home/user/weekly-script
0 45 23 * * * /home/user/nightly-backup";
    tokio::fs::write(&system2_path, system2_content)
        .await
        .expect("Failed to write system2 crontab");

    // Create a hidden file that should be ignored
    let hidden_path = crontabs_dir.join(".hidden");
    tokio::fs::write(&hidden_path, "0 0 * * * * /should/be/ignored")
        .await
        .expect("Failed to write hidden file");

    // Create a backup file that should be ignored
    let backup_path = crontabs_dir.join("backup~");
    tokio::fs::write(&backup_path, "0 0 * * * * /should/also/be/ignored")
        .await
        .expect("Failed to write backup file");

    // Initialize CrontabManager with scheduler path
    let mut manager = CrontabManager::new(Some(scheduler_path.clone()));

    // Load all crontabs
    manager
        .load_all()
        .await
        .expect("Failed to load all crontabs");

    // Verify scheduler crontab entries
    assert_eq!(manager.scheduler_crontab.entries.len(), 2);

    let scheduler_commands: Vec<&str> = manager
        .scheduler_crontab
        .entries
        .iter()
        .map(|entry| entry.command.as_str())
        .collect();
    assert!(scheduler_commands.contains(&"/usr/bin/scheduler-task"));
    assert!(scheduler_commands.contains(&"/usr/bin/backup-task"));

    let scheduler_sources: Vec<Option<&str>> = manager
        .scheduler_crontab
        .entries
        .iter()
        .map(|entry| entry.source.as_deref())
        .collect();
    assert!(scheduler_sources.contains(&Some("scheduler")));
    assert!(scheduler_sources.contains(&Some("scheduler-backup")));

    // Verify system crontabs were loaded (should have 2 system crontabs: root and user)
    assert_eq!(manager.system_crontabs.len(), 2);

    // Check that system crontabs contain the expected entries
    let all_system_entries: Vec<&CronEntry> = manager
        .system_crontabs
        .iter()
        .flat_map(|crontab| &crontab.entries)
        .collect();
    assert_eq!(all_system_entries.len(), 4); // 2 from root + 2 from user

    let system_commands: Vec<&str> = all_system_entries
        .iter()
        .map(|entry| entry.command.as_str())
        .collect();
    assert!(system_commands.contains(&"/usr/bin/system-task"));
    assert!(system_commands.contains(&"/usr/bin/daily-maintenance"));
    assert!(system_commands.contains(&"/home/user/weekly-script"));
    assert!(system_commands.contains(&"/home/user/nightly-backup"));

    // Verify get_all_entries returns all entries from both scheduler and system crontabs
    let all_entries = manager.get_all_entries();
    assert_eq!(all_entries.len(), 6); // 2 scheduler + 4 system

    // Verify get_all_command_entries excludes dummy commands (all our entries are real commands)
    let command_entries = manager.get_all_command_entries();
    assert_eq!(command_entries.len(), 6); // All are real commands, no dummy commands

    // Verify hidden and backup files were ignored
    let all_commands: Vec<&str> = all_entries
        .iter()
        .map(|entry| entry.command.as_str())
        .collect();
    assert!(!all_commands.contains(&"/should/be/ignored"));
    assert!(!all_commands.contains(&"/should/also/be/ignored"));

    // Verify system crontab paths are correct
    let system_paths: Vec<&PathBuf> = manager
        .system_crontabs
        .iter()
        .map(|crontab| &crontab.path)
        .collect();
    assert!(system_paths.contains(&&system1_path));
    assert!(system_paths.contains(&&system2_path));

    // Verify scheduler path is NOT in system crontabs
    assert!(!system_paths.contains(&&scheduler_path));
}
