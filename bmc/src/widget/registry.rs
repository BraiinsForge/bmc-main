// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::path::PathBuf;

use bmc_ipc::SizeType;
use bmc_widget::{Manifest, ManifestError};
use tracing::warn;
use uuid::Uuid;

/// Information about a discovered widget.
#[derive(Debug, Clone)]
pub struct WidgetInfo {
    /// Parsed and validated widget manifest.
    pub manifest: Manifest,
    /// Path to the widget directory containing manifest.json.
    pub widget_dir: PathBuf,
    /// Absolute path to the widget binary.
    pub binary_path: PathBuf,
}

/// Error that can occur during widget registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("failed to read directory '{path}': {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read manifest from '{path}': {source}")]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse manifest from '{path}': {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: ManifestError,
    },

    #[error("binary not found at '{path}'")]
    BinaryNotFound { path: PathBuf },

    #[error("binary at '{path}' is not executable")]
    BinaryNotExecutable { path: PathBuf },
}

/// Registry of available widgets discovered on the system.
#[derive(Debug)]
pub struct WidgetRegistry {
    widgets: HashMap<Uuid, WidgetInfo>,
}

impl WidgetRegistry {
    /// Create a new widget registry from the provided widgets.
    ///
    /// Duplicate UIDs are handled by keeping the first widget and logging a warning.
    pub fn new(widgets: impl IntoIterator<Item = WidgetInfo>) -> Self {
        let mut map = HashMap::new();

        for widget_info in widgets {
            let uid = widget_info.manifest.uid;
            match map.entry(uid) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(widget_info);
                }
                std::collections::hash_map::Entry::Occupied(_) => {
                    warn!(
                        "duplicate widget UID {}, keeping first found, skipping: {}",
                        uid,
                        widget_info.widget_dir.display()
                    );
                }
            }
        }

        Self { widgets: map }
    }

    /// Get a widget by its UID.
    #[must_use]
    pub fn get(&self, uid: &Uuid) -> Option<&WidgetInfo> {
        self.widgets.get(uid)
    }

    /// List all available widgets.
    pub fn list(&self) -> impl Iterator<Item = &WidgetInfo> {
        self.widgets.values()
    }

    /// Check if a widget supports a given size.
    #[must_use]
    pub fn supports_size(&self, uid: &Uuid, size: SizeType) -> bool {
        self.widgets
            .get(uid)
            .is_some_and(|w| w.manifest.sizes.contains(&size))
    }

    /// Returns the number of registered widgets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.widgets.len()
    }

    /// Returns true if no widgets are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_widget::Manifest;

    fn make_widget_info(uid: &str, name: &str, sizes: Vec<SizeType>) -> WidgetInfo {
        let manifest = Manifest {
            uid: Uuid::parse_str(uid).expect("BUG: invalid test UUID"),
            version: semver::Version::new(1, 0, 0),
            name: name.to_owned(),
            description: "Test widget".to_owned(),
            author: None,
            binary: PathBuf::from("bin/widget"),
            settings: vec![],
            sizes,
            params: indexmap::IndexMap::new(),
        };

        WidgetInfo {
            manifest,
            widget_dir: PathBuf::from("/test/widgets").join(name),
            binary_path: PathBuf::from("/test/widgets").join(name).join("bin/widget"),
        }
    }

    #[test]
    fn empty_registry() {
        let registry = WidgetRegistry::new(vec![]);
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn single_widget() {
        let widget = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "test-widget",
            vec![SizeType::Small],
        );

        let registry = WidgetRegistry::new(vec![widget]);
        assert_eq!(registry.len(), 1);

        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")
            .expect("BUG: failed to parse uid");
        let widget = registry.get(&uid).expect("BUG: widget not found");
        assert_eq!(widget.manifest.name, "test-widget");
    }

    #[test]
    fn multiple_widgets() {
        let widget_a = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440001",
            "widget-a",
            vec![SizeType::Small],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![SizeType::Medium],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_uid_keeps_first() {
        let widget_first = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "first-widget",
            vec![SizeType::Small],
        );
        let widget_duplicate = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "duplicate-widget",
            vec![SizeType::Large],
        );

        let registry = WidgetRegistry::new(vec![widget_first, widget_duplicate]);
        assert_eq!(registry.len(), 1);

        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")
            .expect("BUG: failed to parse uid");
        let widget = registry.get(&uid).expect("BUG: widget not found");
        assert_eq!(widget.manifest.name, "first-widget");
    }

    #[test]
    fn get_nonexistent_widget_returns_none() {
        let registry = WidgetRegistry::new(vec![]);
        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440099")
            .expect("BUG: failed to parse uid");
        assert!(registry.get(&uid).is_none());
    }

    #[test]
    fn supports_size_check() {
        let widget = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "test-widget",
            vec![SizeType::Small, SizeType::Medium],
        );

        let registry = WidgetRegistry::new(vec![widget]);
        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")
            .expect("BUG: failed to parse uid");

        assert!(registry.supports_size(&uid, SizeType::Small));
        assert!(registry.supports_size(&uid, SizeType::Medium));
        assert!(!registry.supports_size(&uid, SizeType::Large));
        assert!(!registry.supports_size(&uid, SizeType::Full));
    }

    #[test]
    fn list_widgets() {
        let widget_a = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440001",
            "widget-a",
            vec![SizeType::Small],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![SizeType::Medium],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        let names: Vec<_> = registry.list().map(|w| w.manifest.name.as_str()).collect();

        assert_eq!(names.len(), 2);
        assert!(names.contains(&"widget-a"));
        assert!(names.contains(&"widget-b"));
    }
}
