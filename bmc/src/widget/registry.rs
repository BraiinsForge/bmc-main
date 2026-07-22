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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use bmc_widget_manifest::{Manifest, ManifestError, ViewportShape, WidgetViewportConstraint};
use tracing::warn;
use uuid::Uuid;

use super::{PathDiscovery, WidgetDiscovery};

/// Information about a discovered widget.
#[derive(Debug, Clone)]
pub struct WidgetInfo {
    /// Parsed and validated widget manifest.
    pub manifest: Manifest,
    /// Path to the widget directory containing manifest.json.
    pub widget_dir: PathBuf,
    /// Absolute path to the widget binary.
    pub binary_path: PathBuf,
    /// Resolved icon path when the manifest declares one; served by BMC.
    pub icon_path: Option<PathBuf>,
}

/// Error that can occur during widget registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("failed to read directory '{path}': {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("widget discovery task failed for '{path}': {source}")]
    DiscoveryTask {
        path: PathBuf,
        #[source]
        source: tokio::task::JoinError,
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

/// A concrete widget viewport derived from active hardware and a placement.
/// Matched against a manifest's `supported_viewports` with inclusive ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportDescriptor {
    /// Viewport shape.
    pub viewport_shape: ViewportShape,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Dots per inch.
    pub dpi: u32,
}

impl ViewportDescriptor {
    /// True when `constraint` covers this descriptor: equal viewport shape and
    /// all of width/height/dpi inside the constraint's inclusive ranges.
    /// Missing min/max bounds are open-ended.
    #[must_use]
    pub fn matched_by(&self, constraint: &WidgetViewportConstraint) -> bool {
        self.viewport_shape == constraint.viewport_shape
            && constraint.min_width.is_none_or(|min| self.width >= min)
            && constraint.max_width.is_none_or(|max| self.width <= max)
            && constraint.min_height.is_none_or(|min| self.height >= min)
            && constraint.max_height.is_none_or(|max| self.height <= max)
            && constraint.min_dpi.is_none_or(|min| self.dpi >= min)
            && constraint.max_dpi.is_none_or(|max| self.dpi <= max)
    }
}

/// Map a slot span to its BMC100 derived descriptor. Only 1x1, 2x1, and 2x2
/// are valid in this slice; everything else returns `None`.
#[must_use]
pub fn slot_span_descriptor(columns: u32, rows: u32) -> Option<ViewportDescriptor> {
    let (width, height) = match (columns, rows) {
        (1, 1) => (317, 238),
        (2, 1) => (638, 238),
        (2, 2) => (638, 480),
        _ => return None,
    };
    Some(ViewportDescriptor {
        viewport_shape: ViewportShape::Rectangular,
        width,
        height,
        dpi: 217,
    })
}

/// Registry of available widgets discovered on the system.
///
/// The widget map is interior-mutable so [`WidgetRegistry::refresh`] can
/// re-scan the discovery paths at runtime (e.g. after a widget package is
/// installed) and swap in the new set without rebuilding the shared handle.
#[derive(Debug)]
pub struct WidgetRegistry {
    widgets: RwLock<HashMap<Uuid, WidgetInfo>>,
    /// Discovery paths to re-scan on `refresh`. `None` marks a static
    /// registry built from an explicit widget set (tests); its `refresh`
    /// is a no-op so it is never wiped.
    paths: Option<Vec<PathBuf>>,
}

impl WidgetRegistry {
    /// Create a static widget registry from the provided widgets.
    ///
    /// Duplicate UIDs are handled by keeping the first widget and logging a warning.
    /// The result is not refreshable ([`WidgetRegistry::refresh`] is a no-op).
    pub fn new(widgets: impl IntoIterator<Item = WidgetInfo>) -> Self {
        Self {
            widgets: RwLock::new(Self::build_map(widgets)),
            paths: None,
        }
    }

    /// Discover widgets under `paths` and build a refreshable registry that
    /// remembers those paths for later [`WidgetRegistry::refresh`] calls.
    pub async fn discover(paths: Vec<PathBuf>) -> Self {
        let widgets = PathDiscovery::new(paths.clone()).discover().await;
        Self {
            widgets: RwLock::new(Self::build_map(widgets)),
            paths: Some(paths),
        }
    }

    /// Re-scan the discovery paths and replace the widget set with the result.
    ///
    /// A no-op for a static (`new`-built) registry. Discovery runs without the
    /// lock held; the write lock is taken only for the final swap, so no
    /// `.await` ever happens while holding it.
    pub async fn refresh(&self) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        let widgets = PathDiscovery::new(paths).discover().await;
        let map = Self::build_map(widgets);
        *self
            .widgets
            .write()
            .expect("BUG: widget registry lock poisoned") = map;
    }

    fn build_map(widgets: impl IntoIterator<Item = WidgetInfo>) -> HashMap<Uuid, WidgetInfo> {
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

        map
    }

    /// Get a widget by its UID.
    #[must_use]
    pub fn get(&self, uid: &Uuid) -> Option<WidgetInfo> {
        self.widgets
            .read()
            .expect("BUG: widget registry lock poisoned")
            .get(uid)
            .cloned()
    }

    /// List all available widgets.
    #[must_use]
    pub fn list(&self) -> Vec<WidgetInfo> {
        self.widgets
            .read()
            .expect("BUG: widget registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// True when the widget's manifest declares at least one viewport
    /// constraint covering `descriptor`.
    #[must_use]
    pub fn supports_viewport(&self, uid: &Uuid, descriptor: &ViewportDescriptor) -> bool {
        self.widgets
            .read()
            .expect("BUG: widget registry lock poisoned")
            .get(uid)
            .is_some_and(|w| {
                w.manifest
                    .supported_viewports
                    .iter()
                    .any(|c| descriptor.matched_by(c))
            })
    }

    /// Returns the number of registered widgets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.widgets
            .read()
            .expect("BUG: widget registry lock poisoned")
            .len()
    }

    /// Returns true if no widgets are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.widgets
            .read()
            .expect("BUG: widget registry lock poisoned")
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_widget_manifest::{Manifest, ViewportShape, WidgetCategory, WidgetViewportConstraint};

    fn constraint(
        shape: ViewportShape,
        wmin: u32,
        wmax: u32,
        hmin: u32,
        hmax: u32,
    ) -> WidgetViewportConstraint {
        WidgetViewportConstraint {
            viewport_shape: shape,
            min_width: Some(wmin),
            max_width: Some(wmax),
            min_height: Some(hmin),
            max_height: Some(hmax),
            min_dpi: Some(1),
            max_dpi: Some(1),
        }
    }

    fn make_widget_info(
        uid: &str,
        name: &str,
        viewports: Vec<WidgetViewportConstraint>,
    ) -> WidgetInfo {
        let manifest = Manifest {
            uid: Uuid::parse_str(uid).expect("BUG: invalid test UUID"),
            version: semver::Version::new(1, 0, 0),
            name: name.to_owned(),
            subname: None,
            description: "Test widget".to_owned(),
            config_help: None,
            author: None,
            binary: PathBuf::from("bin/widget"),
            icon: None,
            category: WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: viewports,
            params: indexmap::IndexMap::new(),
        };

        WidgetInfo {
            manifest,
            widget_dir: PathBuf::from("/test/widgets").join(name),
            binary_path: PathBuf::from("/test/widgets").join(name).join("bin/widget"),
            icon_path: None,
        }
    }

    #[tokio::test]
    async fn discovery_task_failure_message_is_not_read_dir_error() {
        let path = PathBuf::from("/test/widgets");
        let source = tokio::task::spawn_blocking(|| panic!("simulated discovery panic"))
            .await
            .expect_err("BUG: blocking task should panic");

        let error = RegistryError::DiscoveryTask {
            path: path.clone(),
            source,
        };
        let message = error.to_string();

        assert!(
            message.starts_with("widget discovery task failed for '/test/widgets':"),
            "message should identify discovery task failure, got {message:?}"
        );
        assert!(
            !message.contains("failed to read directory"),
            "join errors must not be labeled as directory read failures"
        );
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
            vec![constraint(ViewportShape::Rectangular, 317, 317, 238, 238)],
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
            vec![constraint(ViewportShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![constraint(ViewportShape::Rectangular, 638, 638, 238, 238)],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_uid_keeps_first() {
        let widget_first = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "first-widget",
            vec![constraint(ViewportShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_duplicate = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "duplicate-widget",
            vec![constraint(ViewportShape::Rectangular, 638, 638, 480, 480)],
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
    fn list_widgets() {
        let widget_a = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440001",
            "widget-a",
            vec![constraint(ViewportShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![constraint(ViewportShape::Rectangular, 638, 638, 238, 238)],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        let widgets = registry.list();
        let names: Vec<_> = widgets.iter().map(|w| w.manifest.name.as_str()).collect();

        assert_eq!(names.len(), 2);
        assert!(names.contains(&"widget-a"));
        assert!(names.contains(&"widget-b"));
    }

    #[test]
    fn descriptor_inside_inclusive_range_matches() {
        let c = constraint(ViewportShape::Rectangular, 160, 1280, 238, 480);
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(desc.matched_by(&c));
    }

    #[test]
    fn descriptor_outside_range_does_not_match() {
        let c = constraint(ViewportShape::Rectangular, 160, 320, 238, 480);
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(!desc.matched_by(&c));
    }

    #[test]
    fn descriptor_shape_mismatch_does_not_match() {
        let c = constraint(ViewportShape::Round, 480, 480, 480, 480);
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(!desc.matched_by(&c));
    }

    #[test]
    fn omitted_constraint_bounds_are_unbounded() {
        let c = WidgetViewportConstraint {
            viewport_shape: ViewportShape::Rectangular,
            min_width: None,
            max_width: None,
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        };
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 10_000,
            height: 480,
            dpi: 999,
        };
        assert!(desc.matched_by(&c));
    }

    #[test]
    fn allowed_slot_spans_map_to_bmc100_descriptors() {
        assert_eq!(
            slot_span_descriptor(1, 1),
            Some(ViewportDescriptor {
                viewport_shape: ViewportShape::Rectangular,
                width: 317,
                height: 238,
                dpi: 217
            })
        );
        assert_eq!(
            slot_span_descriptor(2, 1),
            Some(ViewportDescriptor {
                viewport_shape: ViewportShape::Rectangular,
                width: 638,
                height: 238,
                dpi: 217
            })
        );
        assert_eq!(
            slot_span_descriptor(2, 2),
            Some(ViewportDescriptor {
                viewport_shape: ViewportShape::Rectangular,
                width: 638,
                height: 480,
                dpi: 217
            })
        );
    }

    #[test]
    fn disallowed_slot_spans_are_rejected() {
        assert_eq!(slot_span_descriptor(1, 2), None);
        assert_eq!(slot_span_descriptor(3, 1), None);
        assert_eq!(slot_span_descriptor(4, 2), None);
    }

    #[test]
    fn supports_viewport_matches_declared_constraint() {
        let widget = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "test-widget",
            vec![constraint(ViewportShape::Rectangular, 317, 317, 238, 238)],
        );
        let registry = WidgetRegistry::new(vec![widget]);
        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: parse uid");
        let desc = ViewportDescriptor {
            viewport_shape: ViewportShape::Rectangular,
            width: 317,
            height: 238,
            dpi: 1,
        };
        assert!(registry.supports_viewport(&uid, &desc));
        let other = ViewportDescriptor {
            width: 1280,
            height: 480,
            ..desc
        };
        assert!(!registry.supports_viewport(&uid, &other));
    }

    fn write_widget(base_dir: &std::path::Path, name: &str, uid: &str) {
        use std::os::unix::fs::PermissionsExt;
        let widget_dir = base_dir.join(name);
        std::fs::create_dir_all(&widget_dir).expect("BUG: create widget dir");
        let manifest = format!(
            r#"{{"uid":"{uid}","version":"1.0.0","name":"{name}","description":"t","binary":"widget","supported_viewports":[{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]}}"#
        );
        std::fs::write(widget_dir.join("manifest.json"), manifest).expect("BUG: write manifest");
        let binary_path = widget_dir.join("widget");
        std::fs::write(&binary_path, "#!/bin/sh\n").expect("BUG: write binary");
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod binary");
    }

    #[tokio::test]
    async fn refresh_picks_up_newly_added_widget() {
        let temp_dir = tempfile::TempDir::new().expect("BUG: tempdir");
        write_widget(
            temp_dir.path(),
            "first",
            "550e8400-e29b-41d4-a716-446655440000",
        );

        let registry = WidgetRegistry::discover(vec![temp_dir.path().to_path_buf()]).await;
        assert_eq!(registry.len(), 1);

        write_widget(
            temp_dir.path(),
            "second",
            "550e8400-e29b-41d4-a716-446655440001",
        );
        registry.refresh().await;

        assert_eq!(registry.len(), 2);
        let second = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("BUG: uid");
        assert!(registry.get(&second).is_some());
    }

    #[tokio::test]
    async fn refresh_drops_removed_widget() {
        let temp_dir = tempfile::TempDir::new().expect("BUG: tempdir");
        write_widget(
            temp_dir.path(),
            "gone",
            "550e8400-e29b-41d4-a716-446655440000",
        );

        let registry = WidgetRegistry::discover(vec![temp_dir.path().to_path_buf()]).await;
        assert_eq!(registry.len(), 1);

        std::fs::remove_dir_all(temp_dir.path().join("gone")).expect("BUG: remove widget");
        registry.refresh().await;

        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn refresh_is_a_noop_for_static_registry() {
        let widget = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "static-widget",
            vec![],
        );
        let registry = WidgetRegistry::new(vec![widget]);
        assert_eq!(registry.len(), 1);

        registry.refresh().await;

        assert_eq!(registry.len(), 1, "static registry must not be wiped");
    }
}
