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
