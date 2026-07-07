// Copyright (C) 2026  Braiins Systems s.r.o.

//! Build the widget directory the mock's registry discovers from, holding
//! only the widgets that are not still shadowed. Installing a widget
//! re-stages the tree so the just-installed widget appears without a
//! restart, matching how a real device gains a widget on install.

use std::collections::BTreeSet;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Widget directories under `root`, mirroring production `PathDiscovery`
/// exactly: a directory holding a `manifest.json`, found by a depth-1..=3 walk
/// that follows links and does not descend past a discovered widget. Yields
/// per-entry walk errors so a caller that gates a mutation on a complete
/// listing can fail loud rather than treat an unreadable tree as empty.
pub(crate) fn widget_dirs(root: &Path) -> Vec<walkdir::Result<PathBuf>> {
    let mut dirs = Vec::new();
    let mut entries = WalkDir::new(root)
        .follow_links(true)
        .min_depth(1)
        .max_depth(3)
        .into_iter();
    while let Some(entry) = entries.next() {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_dir() && entry.path().join("manifest.json").exists() {
                    dirs.push(Ok(entry.path().to_path_buf()));
                    entries.skip_current_dir();
                }
            }
            Err(err) => dirs.push(Err(err)),
        }
    }
    dirs
}

/// Recreate `staging` as symlinks to every widget under `bundle` whose
/// package name is not in `shadowed`. The bundle is enumerated exactly like
/// production discovery (see [`widget_dirs`]) so staged widgets and the offered
/// installable set partition the same widget set. Symlink targets are absolute
/// so discovery resolves them regardless of its working directory, and the tree
/// structure under the bundle is preserved so grouped widgets stay at the depth
/// discovery expects. A traversal error aborts before any symlink is written so
/// an unreadable bundle never masquerades as an empty successful staging.
pub fn stage_installed_widgets(
    bundle: &Path,
    staging: &Path,
    shadowed: &BTreeSet<String>,
) -> io::Result<()> {
    let widget_dirs = widget_dirs(bundle)
        .into_iter()
        .collect::<walkdir::Result<Vec<_>>>()
        .map_err(io::Error::from)?;

    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;

    for widget_dir in widget_dirs {
        let Some(name) = widget_dir.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if shadowed.contains(&format!("widget-{name}")) {
            continue;
        }
        let Ok(relative) = widget_dir.strip_prefix(bundle) else {
            continue;
        };
        let link = staging.join(relative);
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent)?;
        }
        symlink(std::path::absolute(&widget_dir)?, &link)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).expect("BUG: create widget dir");
        std::fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{"uid":"7cb584a8-1f26-42a0-867e-955aadd2391c","version":"1.0.0",
                    "name":"{name}","description":"A {name} widget.","binary":"bin/{name}",
                    "category":"clock","supported_viewports":[]}}"#
            ),
        )
        .expect("BUG: write manifest");
    }

    fn staged_names(staging: &Path) -> BTreeSet<String> {
        widget_dirs(staging)
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|dir| dir.file_name()?.to_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn stages_only_unshadowed_widgets() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("bundle");
        write_manifest(&bundle.join("flip-clock"), "flip-clock");
        write_manifest(&bundle.join("weather"), "weather");
        let staging = dir.path().join("staged");

        stage_installed_widgets(
            &bundle,
            &staging,
            &BTreeSet::from(["widget-flip-clock".to_owned()]),
        )
        .expect("BUG: staging failed");

        assert_eq!(
            staged_names(&staging),
            BTreeSet::from(["weather".to_owned()])
        );
    }

    #[test]
    fn preserves_grouped_widget_depth() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("bundle");
        write_manifest(&bundle.join("group").join("nested"), "nested");
        let staging = dir.path().join("staged");

        stage_installed_widgets(&bundle, &staging, &BTreeSet::new()).expect("BUG: staging failed");

        assert!(
            staging
                .join("group")
                .join("nested")
                .join("manifest.json")
                .exists(),
            "grouped widget must keep its bundle-relative depth"
        );
    }

    #[test]
    fn stages_widget_at_max_discovery_depth() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("bundle");
        // A widget directory three levels deep is the deepest production
        // discovery finds; staging must not miss it.
        write_manifest(&bundle.join("a").join("b").join("deep"), "deep");
        let staging = dir.path().join("staged");

        stage_installed_widgets(&bundle, &staging, &BTreeSet::new()).expect("BUG: staging failed");

        assert_eq!(staged_names(&staging), BTreeSet::from(["deep".to_owned()]));
    }

    #[test]
    fn missing_bundle_is_an_error_not_an_empty_success() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("does-not-exist");
        let staging = dir.path().join("staged");
        // An unreadable bundle must fail loudly so the caller does not persist
        // an install against an empty staged tree.
        assert!(stage_installed_widgets(&bundle, &staging, &BTreeSet::new()).is_err());
    }

    #[test]
    fn symlink_targets_are_absolute() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("bundle");
        write_manifest(&bundle.join("flip-clock"), "flip-clock");
        let staging = dir.path().join("staged");

        stage_installed_widgets(&bundle, &staging, &BTreeSet::new()).expect("BUG: staging failed");

        let link = staging.join("flip-clock");
        let target = std::fs::read_link(&link).expect("BUG: read staged symlink");
        assert!(
            target.is_absolute(),
            "staged symlink target must be absolute: {target:?}"
        );
    }

    #[test]
    fn re_staging_replaces_previous_contents() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let bundle = dir.path().join("bundle");
        write_manifest(&bundle.join("flip-clock"), "flip-clock");
        write_manifest(&bundle.join("weather"), "weather");
        let staging = dir.path().join("staged");

        stage_installed_widgets(
            &bundle,
            &staging,
            &BTreeSet::from(["widget-flip-clock".to_owned(), "widget-weather".to_owned()]),
        )
        .expect("BUG: staging failed");
        assert!(staged_names(&staging).is_empty());

        stage_installed_widgets(&bundle, &staging, &BTreeSet::new()).expect("BUG: staging failed");
        assert_eq!(
            staged_names(&staging),
            BTreeSet::from(["flip-clock".to_owned(), "weather".to_owned()])
        );
    }
}
