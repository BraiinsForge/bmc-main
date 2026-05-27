// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::path::PathBuf;

use bmc_widget_manifest::{DisplayShape, Manifest, ManifestError, WidgetViewportConstraint};
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

/// A concrete widget viewport derived from active hardware and a placement.
/// Matched against a manifest's `supported_viewports` with inclusive ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportDescriptor {
    /// Display shape of the viewport.
    pub shape: DisplayShape,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Dots per inch.
    pub dpi: u32,
}

impl ViewportDescriptor {
    /// True when `constraint` covers this descriptor: equal display type and
    /// all of width/height/dpi inside the constraint's inclusive ranges.
    /// Missing min/max bounds are open-ended.
    #[must_use]
    pub fn matched_by(&self, constraint: &WidgetViewportConstraint) -> bool {
        self.shape == constraint.display_type
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
        shape: DisplayShape::Rectangular,
        width,
        height,
        dpi: 217,
    })
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

    /// True when the widget's manifest declares at least one viewport
    /// constraint covering `descriptor`.
    #[must_use]
    pub fn supports_viewport(&self, uid: &Uuid, descriptor: &ViewportDescriptor) -> bool {
        self.widgets.get(uid).is_some_and(|w| {
            w.manifest
                .supported_viewports
                .iter()
                .any(|c| descriptor.matched_by(c))
        })
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
    use bmc_widget_manifest::{DisplayShape, Manifest, WidgetViewportConstraint};

    fn constraint(
        shape: DisplayShape,
        wmin: u32,
        wmax: u32,
        hmin: u32,
        hmax: u32,
    ) -> WidgetViewportConstraint {
        WidgetViewportConstraint {
            display_type: shape,
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
            description: "Test widget".to_owned(),
            author: None,
            binary: PathBuf::from("bin/widget"),
            settings: vec![],
            supported_viewports: viewports,
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
            vec![constraint(DisplayShape::Rectangular, 317, 317, 238, 238)],
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
            vec![constraint(DisplayShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![constraint(DisplayShape::Rectangular, 638, 638, 238, 238)],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_uid_keeps_first() {
        let widget_first = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "first-widget",
            vec![constraint(DisplayShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_duplicate = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440000",
            "duplicate-widget",
            vec![constraint(DisplayShape::Rectangular, 638, 638, 480, 480)],
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
            vec![constraint(DisplayShape::Rectangular, 317, 317, 238, 238)],
        );
        let widget_b = make_widget_info(
            "550e8400-e29b-41d4-a716-446655440002",
            "widget-b",
            vec![constraint(DisplayShape::Rectangular, 638, 638, 238, 238)],
        );

        let registry = WidgetRegistry::new(vec![widget_a, widget_b]);
        let names: Vec<_> = registry.list().map(|w| w.manifest.name.as_str()).collect();

        assert_eq!(names.len(), 2);
        assert!(names.contains(&"widget-a"));
        assert!(names.contains(&"widget-b"));
    }

    #[test]
    fn descriptor_inside_inclusive_range_matches() {
        let c = constraint(DisplayShape::Rectangular, 160, 1280, 238, 480);
        let desc = ViewportDescriptor {
            shape: DisplayShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(desc.matched_by(&c));
    }

    #[test]
    fn descriptor_outside_range_does_not_match() {
        let c = constraint(DisplayShape::Rectangular, 160, 320, 238, 480);
        let desc = ViewportDescriptor {
            shape: DisplayShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(!desc.matched_by(&c));
    }

    #[test]
    fn descriptor_shape_mismatch_does_not_match() {
        let c = constraint(DisplayShape::Round, 480, 480, 480, 480);
        let desc = ViewportDescriptor {
            shape: DisplayShape::Rectangular,
            width: 480,
            height: 480,
            dpi: 1,
        };
        assert!(!desc.matched_by(&c));
    }

    #[test]
    fn omitted_constraint_bounds_are_unbounded() {
        let c = WidgetViewportConstraint {
            display_type: DisplayShape::Rectangular,
            min_width: None,
            max_width: None,
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        };
        let desc = ViewportDescriptor {
            shape: DisplayShape::Rectangular,
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
                shape: DisplayShape::Rectangular,
                width: 317,
                height: 238,
                dpi: 217
            })
        );
        assert_eq!(
            slot_span_descriptor(2, 1),
            Some(ViewportDescriptor {
                shape: DisplayShape::Rectangular,
                width: 638,
                height: 238,
                dpi: 217
            })
        );
        assert_eq!(
            slot_span_descriptor(2, 2),
            Some(ViewportDescriptor {
                shape: DisplayShape::Rectangular,
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
            vec![constraint(DisplayShape::Rectangular, 317, 317, 238, 238)],
        );
        let registry = WidgetRegistry::new(vec![widget]);
        let uid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("BUG: parse uid");
        let desc = ViewportDescriptor {
            shape: DisplayShape::Rectangular,
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
}
