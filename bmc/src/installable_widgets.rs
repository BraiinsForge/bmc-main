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

use bmc_upgrade::packages::InstallablePackage;
use bmc_widget_manifest::{WidgetCategory, WidgetViewportConstraint};
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallablePreview {
    pub image: String,
    /// Scene size the preview depicts (the `assets.previews` map key). Kept as
    /// a free-form string so a size a newer index introduces still round-trips.
    pub size: String,
}

/// Catalog category of an installable widget, read from a package index.
///
/// Locally-authored manifests only ever carry the known [`WidgetCategory`]
/// values, but an index produced by a newer release may list a category this
/// build does not recognize. Unrecognized (or absent) values become
/// [`Self::Unknown`] so one new category cannot break listing the rest of the
/// catalog — mirroring how the index's strategy hints tolerate unknown values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallableCategory {
    Known(WidgetCategory),
    Unknown,
}

impl<'de> Deserialize<'de> for InstallableCategory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(WidgetCategory::deserialize(
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(&raw),
        )
        .map_or(Self::Unknown, Self::Known))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallableWidget {
    pub package_name: String,
    pub uid: String,
    pub version: String,
    pub display_name: String,
    pub subname: Option<String>,
    pub category: InstallableCategory,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub previews: Vec<InstallablePreview>,
    pub supported_viewports: Vec<WidgetViewportConstraint>,
}

#[must_use]
pub fn from_packages(packages: Vec<InstallablePackage>) -> Vec<InstallableWidget> {
    let widget_str = |resolved: &InstallablePackage, key: &str| {
        resolved
            .metadata
            .get("widget")
            .and_then(|w| w.get(key))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    };
    packages
        .into_iter()
        .filter_map(|resolved| {
            if resolved.category.as_deref() != Some("widget") {
                return None;
            }
            // `uid` is load-bearing (the frontend places the widget into a
            // scene by it); a widget missing it is useless, so drop it rather
            // than publish an empty uid.
            let uid = widget_str(&resolved, "uid")?;
            Some(InstallableWidget {
                uid,
                display_name: widget_str(&resolved, "display_name")
                    .unwrap_or_else(|| resolved.name.clone()),
                subname: widget_str(&resolved, "subname"),
                category: resolved
                    .metadata
                    .get("widget")
                    .and_then(|w| w.get("category"))
                    .and_then(|c| InstallableCategory::deserialize(c).ok())
                    .unwrap_or(InstallableCategory::Unknown),
                icon: resolved
                    .metadata
                    .get("assets")
                    .and_then(|a| a.get("icon"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                previews: resolved
                    .metadata
                    .get("assets")
                    .and_then(|a| a.get("previews"))
                    .and_then(serde_json::Value::as_object)
                    .map(|by_size| {
                        by_size
                            .iter()
                            .filter_map(|(size, image)| {
                                image.as_str().map(|image| InstallablePreview {
                                    image: image.to_owned(),
                                    size: size.clone(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                supported_viewports: resolved
                    .metadata
                    .get("widget")
                    .and_then(|widget| widget.get("supported_viewports"))
                    .and_then(|value| Vec::<WidgetViewportConstraint>::deserialize(value).ok())
                    .unwrap_or_default(),
                package_name: resolved.name,
                version: resolved.version,
                description: resolved.description,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn merged_with(
        entries: &[(&str, &str, Option<serde_json::Value>)],
    ) -> bmc_nix::types::MergedIndex {
        let packages: Vec<String> = entries
            .iter()
            .map(|(name, category, metadata)| {
                let meta = metadata
                    .clone()
                    .map_or_else(|| "{}".to_owned(), |m| m.to_string());
                format!(
                    r#"{{"name":"{name}","version":"1.0.0","store_path":"/nix/store/{name}","category":"{category}","metadata":{meta}}}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
            packages.join(",")
        );
        let raw: bmc_nix::types::PackageIndex =
            serde_json::from_str(&json).expect("BUG: parse index");
        bmc_nix::index::merge_indexes(vec![bmc_nix::types::FetchedIndex {
            server_id: "srv".to_owned(),
            server_priority: 10,
            index: raw,
        }])
    }

    fn installable_widgets_from(
        merged: &bmc_nix::types::MergedIndex,
        installed: &std::collections::BTreeSet<String>,
    ) -> Vec<InstallableWidget> {
        from_packages(bmc_upgrade::packages::installable_packages_from(
            merged, installed,
        ))
    }
    #[test]
    fn installable_widgets_keeps_uninstalled_widget_category_only() {
        let merged = merged_with(&[
            (
                "widget-weather",
                "widget",
                Some(serde_json::json!({
                    "widget": {"uid": "uid-weather", "display_name": "Weather", "subname": "Forecast", "category": "info"},
                    "assets": {"icon": "/nix/store/widget-weather/lib/bmc-widgets/weather/icon.svg"}
                })),
            ),
            (
                "widget-clock",
                "widget",
                Some(serde_json::json!({
                    "widget": {"uid": "uid-clock", "display_name": "Clock", "category": "clock"}
                })),
            ),
            ("core", "system", None),
        ]);
        let installed: std::collections::BTreeSet<String> =
            ["widget-clock".to_owned(), "core".to_owned()]
                .into_iter()
                .collect();

        let widgets = installable_widgets_from(&merged, &installed);

        assert_eq!(widgets.len(), 1, "only the uninstalled widget survives");
        let w = &widgets[0];
        assert_eq!(w.package_name, "widget-weather");
        assert_eq!(w.uid, "uid-weather");
        assert_eq!(w.display_name, "Weather");
        assert_eq!(w.subname.as_deref(), Some("Forecast"));
        // "info" is not a category this build knows, so it folds to Unknown
        // rather than dropping the widget from the catalog.
        assert_eq!(w.category, InstallableCategory::Unknown);
        assert_eq!(
            w.icon.as_deref(),
            Some("/nix/store/widget-weather/lib/bmc-widgets/weather/icon.svg")
        );
        // No `assets.previews` in the index, so the preview list defaults empty.
        assert!(w.previews.is_empty());
    }

    #[test]
    fn installable_widgets_read_supported_viewports_from_index() {
        let merged = merged_with(&[(
            "widget-fullscreen",
            "widget",
            Some(serde_json::json!({
                "widget": {
                    "uid": "uid-fullscreen",
                    "supported_viewports": [{
                        "type": "rectangular",
                        "min_width": 1280,
                        "max_width": 1280,
                        "min_height": 480,
                        "max_height": 480
                    }]
                }
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert_eq!(
            widgets[0].supported_viewports,
            vec![WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(1280),
                max_width: Some(1280),
                min_height: Some(480),
                max_height: Some(480),
                min_dpi: None,
                max_dpi: None,
            }]
        );
    }

    #[test]
    fn installable_widgets_default_missing_supported_viewports_to_empty() {
        let merged = merged_with(&[(
            "widget-legacy",
            "widget",
            Some(serde_json::json!({"widget": {"uid": "uid-legacy"}})),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert!(widgets[0].supported_viewports.is_empty());
    }

    #[test]
    fn installable_widgets_default_invalid_supported_viewports_to_empty() {
        let merged = merged_with(&[(
            "widget-invalid",
            "widget",
            Some(serde_json::json!({
                "widget": {"uid": "uid-invalid", "supported_viewports": "full"}
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert!(widgets[0].supported_viewports.is_empty());
    }

    #[test]
    fn installable_widgets_reads_previews_from_index() {
        // Preview art lives under `assets.previews` in the index (a not-yet
        // installed widget has no parsed manifest to read it from), keyed by the
        // scene size it depicts; each entry becomes one `InstallablePreview`.
        let merged = merged_with(&[(
            "widget-weather",
            "widget",
            Some(serde_json::json!({
                "widget": {"uid": "uid-weather", "display_name": "Weather", "category": "weather"},
                "assets": {
                    "icon": "/nix/store/w/icon.svg",
                    "previews": {
                        "full": "https://example.test/weather-full.png",
                        "medium": "https://example.test/weather-medium.png"
                    }
                }
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert_eq!(widgets.len(), 1);
        let by_size: std::collections::BTreeMap<&str, &str> = widgets[0]
            .previews
            .iter()
            .map(|p| (p.size.as_str(), p.image.as_str()))
            .collect();
        assert_eq!(
            by_size,
            std::collections::BTreeMap::from([
                ("full", "https://example.test/weather-full.png"),
                ("medium", "https://example.test/weather-medium.png"),
            ])
        );
    }

    #[test]
    fn installable_category_deserializes_known_and_unknown() {
        let known: InstallableCategory =
            serde_json::from_value(serde_json::json!("weather")).expect("BUG: known category");
        assert_eq!(known, InstallableCategory::Known(WidgetCategory::Weather));

        // A category value a newer index might carry that this build predates.
        let unknown: InstallableCategory =
            serde_json::from_value(serde_json::json!("teleportation")).expect("BUG: unknown ok");
        assert_eq!(unknown, InstallableCategory::Unknown);
    }

    #[test]
    fn installable_widgets_drops_widget_without_uid() {
        // `uid` is load-bearing; a widget package whose metadata lacks it must
        // not be offered, rather than surfacing with an empty uid.
        let merged = merged_with(&[(
            "widget-broken",
            "widget",
            Some(serde_json::json!({
                "widget": {"display_name": "Broken", "category": "info"}
            })),
        )]);
        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());
        assert!(widgets.is_empty(), "a widget without a uid must be dropped");
    }
}
