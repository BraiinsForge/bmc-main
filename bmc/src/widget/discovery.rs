// Copyright (C) 2026  Braiins Systems s.r.o.

use std::future::Future;
use std::path::{Path, PathBuf};

use bmc_widget_manifest::Manifest;
use tokio::fs;
use tracing::warn;

use super::{RegistryError, WidgetInfo};

/// Whether `path` names an image we are willing to serve as a widget icon.
/// The icon is browser-rendered (web UI widget picker), so any `image/*` type
/// is fine — svg included. Uses the same `mime_guess` machinery the HTTP server
/// uses to label the response, so "served as an image" and "accepted here" agree.
fn is_image_path(path: &Path) -> bool {
    mime_guess::from_path(path)
        .first()
        .is_some_and(|mime| mime.type_() == mime_guess::mime::IMAGE)
}

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

            let is_widget_dir = if file_type.is_dir() {
                true
            } else if file_type.is_symlink() {
                match fs::metadata(&widget_dir).await {
                    Ok(metadata) => metadata.is_dir(),
                    Err(e) => {
                        warn!(
                            "failed to follow widget directory symlink '{}': {}",
                            widget_dir.display(),
                            e
                        );
                        false
                    }
                }
            } else {
                false
            };

            if !is_widget_dir {
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

        // Relative resolves against the widget dir, absolute wins, like `binary`.
        // The manifest is install-time trusted, but only image-extension paths are
        // served — so a stray `icon = "/etc/shadow"` can never reach the icon endpoint.
        let icon_path = manifest.icon.as_ref().and_then(|icon| {
            let path = widget_dir.join(icon);
            if is_image_path(&path) {
                Some(path)
            } else {
                warn!("widget icon is not an image, ignoring: {}", path.display());
                None
            }
        });

        Ok(WidgetInfo {
            manifest,
            widget_dir: widget_dir.to_path_buf(),
            binary_path,
            icon_path,
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

    fn create_widget_with_icon(base_dir: &Path, name: &str, uid: &str, icon: &str) -> PathBuf {
        let widget_dir = base_dir.join(name);
        std_fs::create_dir_all(&widget_dir).expect("BUG: failed to create widget dir");

        let manifest = format!(
            r#"{{
                "uid": "{uid}",
                "version": "1.0.0",
                "name": "{name}",
                "description": "Test widget",
                "binary": "widget",
                "icon": "{icon}",
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
    async fn discover_resolves_relative_and_absolute_icon() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let rel_dir = create_widget_with_icon(
            temp_dir.path(),
            "rel-widget",
            "550e8400-e29b-41d4-a716-446655440000",
            "assets/icon.svg",
        );
        create_widget_with_icon(
            temp_dir.path(),
            "abs-widget",
            "550e8400-e29b-41d4-a716-446655440001",
            "/usr/share/bmc/icons/abs.png",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        let rel = widgets
            .iter()
            .find(|w| w.manifest.name == "rel-widget")
            .expect("BUG: rel-widget should be discovered");
        assert_eq!(rel.icon_path, Some(rel_dir.join("assets/icon.svg")));

        let abs = widgets
            .iter()
            .find(|w| w.manifest.name == "abs-widget")
            .expect("BUG: abs-widget should be discovered");
        assert_eq!(
            abs.icon_path,
            Some(PathBuf::from("/usr/share/bmc/icons/abs.png")),
            "absolute icon path is used as-is"
        );
    }

    #[tokio::test]
    async fn discover_rejects_non_image_icon() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        create_widget_with_icon(
            temp_dir.path(),
            "shady-widget",
            "550e8400-e29b-41d4-a716-446655440003",
            "/etc/shadow",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert!(
            widgets[0].icon_path.is_none(),
            "a manifest icon that is not an image must not become a served path"
        );
    }

    #[tokio::test]
    async fn discover_widget_without_icon_has_none() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        create_valid_widget(
            temp_dir.path(),
            "no-icon",
            "550e8400-e29b-41d4-a716-446655440002",
        );

        let discovery = PathDiscovery::new(vec![temp_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert!(widgets[0].icon_path.is_none());
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
    async fn discover_symlinked_widget_directory() {
        let scan_dir = TempDir::new().expect("BUG: failed to create scan dir");
        let target_dir = TempDir::new().expect("BUG: failed to create target dir");
        let widget_dir = create_valid_widget(
            target_dir.path(),
            "linked-widget",
            "550e8400-e29b-41d4-a716-446655440003",
        );
        let link_path = scan_dir.path().join("linked-widget");
        std::os::unix::fs::symlink(&widget_dir, &link_path)
            .expect("BUG: failed to create widget symlink");

        let discovery = PathDiscovery::new(vec![scan_dir.path().to_path_buf()]);
        let widgets = discovery.discover().await;

        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].manifest.name, "linked-widget");
        assert_eq!(widgets[0].widget_dir, link_path);
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
