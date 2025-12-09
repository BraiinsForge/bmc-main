// Copyright (C) 2025  Braiins Systems s.r.o.

use std::future::Future;
use std::path::{Path, PathBuf};

use bmc_widget::Manifest;
use tokio::fs;
use tracing::warn;

use super::{RegistryError, WidgetInfo};

/// Trait for platform-specific widget discovery.
///
/// This trait abstracts how widgets are discovered and loaded across different platforms.
pub trait WidgetDiscovery {
    /// Discovers and returns all available widgets.
    ///
    /// Implementations should handle errors gracefully - invalid widgets should be
    /// skipped with a warning rather than failing the entire discovery process.
    fn discover(&self) -> impl Future<Output = Vec<WidgetInfo>> + Send;
}

/// Filesystem-based widget discovery.
///
/// Scans the provided directory paths for widget subdirectories,
/// loads and validates manifests, and returns all valid widgets found.
#[derive(Debug)]
pub struct PathDiscovery {
    paths: Vec<PathBuf>,
}

impl PathDiscovery {
    /// Create a new path-based discovery with the given paths to scan.
    #[must_use]
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    async fn scan_directory(path: &Path) -> Result<Vec<WidgetInfo>, RegistryError> {
        let mut widgets = Vec::new();

        let mut entries = fs::read_dir(path)
            .await
            .map_err(|e| RegistryError::ReadDir {
                path: path.to_path_buf(),
                source: e,
            })?;

        while let Some(entry) = entries.next_entry().await.transpose() {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("failed to read directory entry: {}", e);
                    continue;
                }
            };

            let widget_dir = entry.path();
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(e) => {
                    warn!("failed to get file type: {}", e);
                    continue;
                }
            };

            if !file_type.is_dir() {
                continue;
            }

            match Self::load_widget(&widget_dir).await {
                Ok(info) => widgets.push(info),
                Err(e) => {
                    warn!(
                        "skipping invalid widget at '{}': {}",
                        widget_dir.display(),
                        e
                    );
                }
            }
        }

        Ok(widgets)
    }

    async fn load_widget(widget_dir: &Path) -> Result<WidgetInfo, RegistryError> {
        let manifest_path = widget_dir.join("manifest.json");

        let manifest_content =
            fs::read_to_string(&manifest_path)
                .await
                .map_err(|e| RegistryError::ReadManifest {
                    path: manifest_path.clone(),
                    source: e,
                })?;

        let manifest: Manifest =
            manifest_content
                .parse()
                .map_err(|e| RegistryError::ParseManifest {
                    path: manifest_path,
                    source: e,
                })?;

        let binary_path = widget_dir.join(&manifest.binary);

        if !fs::try_exists(&binary_path)
            .await
            .map_err(|e| RegistryError::ReadManifest {
                path: binary_path.clone(),
                source: e,
            })?
        {
            return Err(RegistryError::BinaryNotFound { path: binary_path });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata =
                fs::metadata(&binary_path)
                    .await
                    .map_err(|e| RegistryError::ReadManifest {
                        path: binary_path.clone(),
                        source: e,
                    })?;
            let permissions = metadata.permissions();
            if permissions.mode() & 0o111 == 0 {
                return Err(RegistryError::BinaryNotExecutable { path: binary_path });
            }
        }

        Ok(WidgetInfo {
            manifest,
            widget_dir: widget_dir.to_path_buf(),
            binary_path,
        })
    }
}

impl WidgetDiscovery for PathDiscovery {
    async fn discover(&self) -> Vec<WidgetInfo> {
        let mut widgets = Vec::new();

        for scan_path in &self.paths {
            if !scan_path.exists() {
                warn!("widget scan path does not exist: {}", scan_path.display());
                continue;
            }

            match Self::scan_directory(scan_path).await {
                Ok(mut discovered) => {
                    widgets.append(&mut discovered);
                }
                Err(e) => {
                    warn!("failed to scan widget directory: {}", e);
                }
            }
        }

        widgets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as std_fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn create_valid_widget(base_dir: &Path, name: &str, uid: &str) -> PathBuf {
        let widget_dir = base_dir.join(name);
        std_fs::create_dir_all(&widget_dir).expect("BUG: failed to create widget dir");

        let manifest = format!(
            r#"{{
                "uid": "{uid}",
                "version": "1.0.0",
                "name": "{name}",
                "description": "Test widget",
                "binary": "widget",
                "sizes": ["small"]
            }}"#
        );

        std_fs::write(widget_dir.join("manifest.json"), manifest)
            .expect("BUG: failed to write manifest");

        let binary_path = widget_dir.join("widget");
        std_fs::write(&binary_path, "#!/bin/sh\necho test").expect("BUG: failed to write binary");
        std_fs::set_permissions(&binary_path, std_fs::Permissions::from_mode(0o755))
            .expect("BUG: failed to set permissions");

        widget_dir
    }

    #[tokio::test]
    async fn discover_empty_directory() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);

        let widgets = discovery.discover().await;
        assert!(widgets.is_empty());
    }

    #[tokio::test]
    async fn discover_nonexistent_path() {
        let discovery = PathDiscovery::new(vec![PathBuf::from("/nonexistent/path")]);

        let widgets = discovery.discover().await;
        assert!(widgets.is_empty());
    }

    #[tokio::test]
    async fn discover_single_widget() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        create_valid_widget(
            temp_dir.path(),
            "test-widget",
            "550e8400-e29b-41d4-a716-446655440000",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].manifest.name, "test-widget");
    }

    #[tokio::test]
    async fn discover_multiple_widgets() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        create_valid_widget(
            temp_dir.path(),
            "widget-a",
            "550e8400-e29b-41d4-a716-446655440001",
        );
        create_valid_widget(
            temp_dir.path(),
            "widget-b",
            "550e8400-e29b-41d4-a716-446655440002",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 2);
        let names: Vec<_> = widgets.iter().map(|w| w.manifest.name.as_str()).collect();
        assert!(names.contains(&"widget-a"));
        assert!(names.contains(&"widget-b"));
    }

    #[tokio::test]
    async fn discover_from_multiple_paths() {
        let temp_dir_a = TempDir::new().expect("BUG: failed to create temp dir");
        let temp_dir_b = TempDir::new().expect("BUG: failed to create temp dir");

        create_valid_widget(
            temp_dir_a.path(),
            "widget-a",
            "550e8400-e29b-41d4-a716-446655440001",
        );
        create_valid_widget(
            temp_dir_b.path(),
            "widget-b",
            "550e8400-e29b-41d4-a716-446655440002",
        );

        let discovery = PathDiscovery::new(vec![
            temp_dir_a.path().to_path_buf(),
            temp_dir_b.path().to_path_buf(),
        ]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 2);
    }

    #[tokio::test]
    async fn skip_invalid_manifest() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");

        // Create valid widget
        create_valid_widget(
            temp_dir.path(),
            "valid-widget",
            "550e8400-e29b-41d4-a716-446655440001",
        );

        // Create invalid widget (bad JSON)
        let invalid_dir = temp_dir.path().join("invalid-widget");
        std_fs::create_dir_all(&invalid_dir).expect("BUG: failed to create dir");
        std_fs::write(invalid_dir.join("manifest.json"), "{ invalid json }")
            .expect("BUG: failed to write");

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].manifest.name, "valid-widget");
    }

    #[tokio::test]
    async fn skip_missing_binary() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");

        // Create widget without binary
        let widget_dir = temp_dir.path().join("no-binary-widget");
        std_fs::create_dir_all(&widget_dir).expect("BUG: failed to create dir");
        std_fs::write(
            widget_dir.join("manifest.json"),
            r#"{
                "uid": "550e8400-e29b-41d4-a716-446655440001",
                "version": "1.0.0",
                "name": "no-binary-widget",
                "description": "Test",
                "binary": "widget",
                "sizes": ["small"]
            }"#,
        )
        .expect("BUG: failed to write manifest");

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert!(widgets.is_empty());
    }

    #[tokio::test]
    async fn skip_non_executable_binary() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");

        let widget_dir = temp_dir.path().join("non-exec-widget");
        std_fs::create_dir_all(&widget_dir).expect("BUG: failed to create dir");
        std_fs::write(
            widget_dir.join("manifest.json"),
            r#"{
                "uid": "550e8400-e29b-41d4-a716-446655440001",
                "version": "1.0.0",
                "name": "non-exec-widget",
                "description": "Test",
                "binary": "widget",
                "sizes": ["small"]
            }"#,
        )
        .expect("BUG: failed to write manifest");

        // Create binary without execute permission
        let binary_path = widget_dir.join("widget");
        std_fs::write(&binary_path, "#!/bin/sh").expect("BUG: failed to write binary");
        std_fs::set_permissions(&binary_path, std_fs::Permissions::from_mode(0o644))
            .expect("BUG: failed to set permissions");

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert!(widgets.is_empty());
    }

    #[tokio::test]
    async fn skip_files_in_scan_directory() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");

        // Create a valid widget
        create_valid_widget(
            temp_dir.path(),
            "valid-widget",
            "550e8400-e29b-41d4-a716-446655440001",
        );

        // Create a file (not directory) in scan path - should be ignored
        std_fs::write(temp_dir.path().join("some-file.txt"), "not a widget")
            .expect("BUG: failed to write file");

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].manifest.name, "valid-widget");
    }

    #[tokio::test]
    async fn widget_paths_are_correct() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let widget_dir = create_valid_widget(
            temp_dir.path(),
            "test-widget",
            "550e8400-e29b-41d4-a716-446655440000",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].widget_dir, widget_dir);
        assert_eq!(widgets[0].binary_path, widget_dir.join("widget"));
    }
}
