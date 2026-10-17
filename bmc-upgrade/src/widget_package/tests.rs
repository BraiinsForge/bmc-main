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

use super::fixtures::*;
use super::*;
use nix_archive::nar::{CaseHack, restore};

/// Unpack `tree` the way the device does and validate it.
fn validate(tree: &Node) -> Result<WidgetPackage, WidgetPackageError> {
    let staging = tempfile::tempdir().expect("BUG: tempdir");
    let root = staging.path().join("package");
    restore(&encode(tree), &root, CaseHack::Disabled).expect("BUG: fixture restores");
    WidgetPackage::from_dir(&root, SDK_MAJOR)
}

fn validate_dir(dir: BTreeMap<String, Node>) -> Result<WidgetPackage, WidgetPackageError> {
    validate(&package_tree("example", dir))
}

#[test]
fn a_built_widget_package_is_accepted() {
    let package = validate(&package_tree("example", widget_dir("example", "1.2.3")))
        .expect("BUG: the fixture mirrors nix/wasm-widgets.nix");
    assert_eq!(package.name(), "example");
    assert_eq!(package.package_name(), "widget-example");
    assert_eq!(package.manifest().version.to_string(), "1.2.3");
}

#[test]
fn the_resolved_package_is_a_user_installed_local_widget() {
    let package = validate(&package_tree("example", widget_dir("example", "1.2.3")))
        .expect("BUG: valid fixture");
    let resolved = package.resolved("/nix/store/abc-bmc-widget-example".to_owned());
    assert_eq!(resolved.name, "widget-example");
    assert_eq!(resolved.version, "1.2.3");
    assert_eq!(resolved.category.as_deref(), Some("widget"));
    assert_eq!(resolved.installed_by, InstalledBy::User);
    assert_eq!(resolved.installed_from, "local");
}

#[test]
fn a_wrapper_that_differs_from_the_template_in_any_byte_is_refused() {
    let template = expected_wrapper("example", SDK_MAJOR);
    let cases = [
        // sh deletes a backslash-newline, so this runs `exec/run/current-profile/…`.
        template.replacen("exec /run", "exec\\\n/run", 1),
        template.replace(" \\\n    ", " "),
        format!("{template}\n"),
        format!(
            "#!/bin/sh\nexec /run/current-profile/bin/bmc-wasm-thin-v{SDK_MAJOR} --wasm /nix/store/x/example.wasm \"$@\"\n"
        ),
    ];
    for wrapper in cases {
        let mut dir = widget_dir("example", "1.0.0");
        set(&mut dir, "bin/example", executable(wrapper.as_bytes()));
        assert!(
            matches!(
                validate_dir(dir),
                Err(WidgetPackageError::Wrapper(path)) if path == "bin/example"
            ),
            "only the exact bytes nix writes are known to run the launcher: {wrapper:?}"
        );
    }
}

#[test]
fn a_wrapper_for_another_sdk_major_names_both_versions() {
    let mut dir = widget_dir("example", "1.0.0");
    set(
        &mut dir,
        "bin/example",
        executable(expected_wrapper("example", SDK_MAJOR + 1).as_bytes()),
    );
    assert!(matches!(
        validate_dir(dir),
        Err(WidgetPackageError::SdkMajor { package, device })
            if package == SDK_MAJOR + 1 && device == SDK_MAJOR
    ));
}

#[test]
fn an_oversized_upload_fails_as_it_streams() {
    use tokio::io::AsyncReadExt as _;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("BUG: runtime");
    let drain = |len: u64| {
        let mut sink = tokio::io::sink();
        runtime.block_on(tokio::io::copy(
            &mut limit_archive(Box::pin(tokio::io::repeat(0).take(len))),
            &mut sink,
        ))
    };
    let error = drain(MAX_ARCHIVE_BYTES + 1).expect_err("BUG: one byte over fails");
    assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
    assert_eq!(
        drain(MAX_ARCHIVE_BYTES).expect("BUG: the limit itself is allowed"),
        MAX_ARCHIVE_BYTES
    );
}

#[test]
fn a_native_widget_is_named_as_such() {
    let mut dir = widget_dir("example", "1.0.0");
    set(
        &mut dir,
        "bin/example",
        executable(b"\x7fELF\x01\x01\x01 rest"),
    );
    remove(&mut dir, "lib/wasm");
    assert!(matches!(
        validate_dir(dir),
        Err(WidgetPackageError::NativeBinary(path)) if path == "bin/example"
    ));
}

#[test]
fn unrelated_content_is_refused_by_path() {
    let tree = dir(vec![("bin", dir(vec![("sh", executable(b"#!/bin/sh\n"))]))]);
    assert!(matches!(
        validate(&tree),
        Err(WidgetPackageError::NotOneWidget(found)) if found == "bin"
    ));

    let mut widget = widget_dir("example", "1.0.0");
    set(&mut widget, "lib/libnative.so", file(b"\x7fELF"));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Unexpected(path)) if path == "lib/libnative.so"
    ));
}

#[test]
fn two_widgets_in_one_archive_are_refused() {
    let tree = dir(vec![(
        "lib",
        dir(vec![(
            "bmc-widgets",
            dir(vec![
                ("a", Node::Directory(widget_dir("a", "1.0.0"))),
                ("b", Node::Directory(widget_dir("b", "1.0.0"))),
            ]),
        )]),
    )]);
    assert!(matches!(
        validate(&tree),
        Err(WidgetPackageError::NotOneWidget(found)) if found == "a, b"
    ));
}

#[test]
fn a_widget_under_a_maintainer_folder_is_named_after_both() {
    let package = validate(&maintained_package_tree(
        "acme",
        "example",
        widget_dir("example", "1.2.3"),
    ))
    .expect("BUG: a maintainer folder is a valid layout");
    assert_eq!(package.name(), "example");
    assert_eq!(
        package.package_name(),
        "widget-acme_example",
        "the maintainer keeps the name clear of the feed's widget-example"
    );
    assert_eq!(package.relative_dir(), Path::new("acme/example"));
    assert_eq!(
        package.resolved("/nix/store/abc".to_owned()).name,
        "widget-acme_example"
    );
}

#[test]
fn a_hyphen_cannot_move_the_maintainer_boundary() {
    let package = |maintainer, name| {
        validate(&maintained_package_tree(
            maintainer,
            name,
            widget_dir(name, "1.0.0"),
        ))
        .expect("BUG: a maintainer folder is a valid layout")
        .package_name()
    };
    assert_ne!(
        package("acme", "foo-bar"),
        package("acme-foo", "bar"),
        "two widget folders sharing a package would replace each other"
    );
}

#[test]
fn a_flat_package_sits_directly_below_the_widgets_root() {
    let package = validate(&package_tree("example", widget_dir("example", "1.0.0")))
        .expect("BUG: valid fixture");
    assert_eq!(package.relative_dir(), Path::new("example"));
}

#[test]
fn a_maintainer_folder_holds_exactly_one_widget() {
    let two = dir(vec![
        ("a", Node::Directory(widget_dir("a", "1.0.0"))),
        ("b", Node::Directory(widget_dir("b", "1.0.0"))),
    ]);
    let tree = dir(vec![(
        "lib",
        dir(vec![("bmc-widgets", dir(vec![("acme", two)]))]),
    )]);
    assert!(matches!(
        validate(&tree),
        Err(WidgetPackageError::NotOneWidget(found)) if found == "acme/a, acme/b"
    ));

    let empty = dir(vec![(
        "lib",
        dir(vec![("bmc-widgets", dir(vec![("acme", dir(vec![]))]))]),
    )]);
    assert!(matches!(
        validate(&empty),
        Err(WidgetPackageError::Missing(path)) if path == "manifest.json"
    ));
}

#[test]
fn a_widget_folder_without_a_manifest_is_missing_it_not_a_maintainer() {
    let mut widget = widget_dir("example", "1.0.0");
    remove(&mut widget, "manifest.json");
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Missing(path)) if path == "manifest.json"
    ));
}

#[test]
fn two_maintainer_folders_are_refused() {
    let tree = dir(vec![(
        "lib",
        dir(vec![(
            "bmc-widgets",
            dir(vec![
                (
                    "acme",
                    dir(vec![("a", Node::Directory(widget_dir("a", "1.0.0")))]),
                ),
                (
                    "other",
                    dir(vec![("b", Node::Directory(widget_dir("b", "1.0.0")))]),
                ),
            ]),
        )]),
    )]);
    assert!(matches!(
        validate(&tree),
        Err(WidgetPackageError::NotOneWidget(found)) if found == "acme, other"
    ));
}

#[test]
fn the_maintainer_folder_name_is_a_slug() {
    for maintainer in ["Acme", "ac me", "-acme", "a_b"] {
        let tree = maintained_package_tree(maintainer, "example", widget_dir("example", "1.0.0"));
        assert!(
            matches!(validate(&tree), Err(WidgetPackageError::BadWidgetName(name)) if name == maintainer),
            "{maintainer:?} must be refused"
        );
    }

    let tree = maintained_package_tree("acme", "Example", widget_dir("Example", "1.0.0"));
    assert!(matches!(
        validate(&tree),
        Err(WidgetPackageError::BadWidgetName(name)) if name == "Example"
    ));
}

#[test]
fn a_bare_file_is_not_a_package() {
    assert!(matches!(
        validate(&file(b"\0asm\x01\0\0\0")),
        Err(WidgetPackageError::NotADirectory(path)) if path.is_empty()
    ));
}

#[test]
fn the_widget_directory_name_is_a_slug() {
    let overlong = "a".repeat(MAX_WIDGET_NAME_LEN + 1);
    for name in ["Example", "ex ample", "-x", "a_b", overlong.as_str()] {
        let tree = dir(vec![(
            "lib",
            dir(vec![(
                "bmc-widgets",
                dir(vec![(name, Node::Directory(widget_dir(name, "1.0.0")))]),
            )]),
        )]);
        assert!(
            matches!(validate(&tree), Err(WidgetPackageError::BadWidgetName(_))),
            "{name:?} must be refused"
        );
    }
}

#[test]
fn symlinks_are_refused_wherever_they_sit() {
    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "assets/link.svg",
        Node::Symlink {
            target: b"/etc/passwd".to_vec(),
        },
    );
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Symlink(path)) if path == "assets/link.svg"
    ));

    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "manifest.json",
        Node::Symlink {
            target: b"nowhere".to_vec(),
        },
    );
    assert!(
        matches!(
            validate_dir(widget),
            Err(WidgetPackageError::Symlink(path)) if path == "manifest.json"
        ),
        "a dangling manifest symlink is a symlink, not a maintainer folder"
    );
}

#[test]
fn an_invalid_manifest_reports_the_manifest_error() {
    let mut widget = widget_dir("example", "1.0.0");
    set(&mut widget, "manifest.json", file(b"{\"uid\": 1}"));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Manifest(_))
    ));
}

#[test]
fn an_overlong_manifest_is_refused_before_it_is_parsed() {
    let mut widget = widget_dir("example", "1.0.0");
    let padded = format!(
        "{}{}",
        manifest_json("example", "1.0.0"),
        " ".repeat(MAX_MANIFEST_BYTES)
    );
    set(&mut widget, "manifest.json", file(padded.as_bytes()));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::FileTooLarge { path, limit })
            if path == "manifest.json" && limit == MAX_MANIFEST_BYTES
    ));
}

#[test]
fn an_overlong_wrapper_is_refused_before_it_is_compared() {
    let mut widget = widget_dir("example", "1.0.0");
    let padded = format!(
        "{}{}",
        expected_wrapper("example", SDK_MAJOR),
        "\n".repeat(MAX_WRAPPER_BYTES)
    );
    set(&mut widget, "bin/example", executable(padded.as_bytes()));
    assert!(
        matches!(
            validate_dir(widget),
            Err(WidgetPackageError::FileTooLarge { path, limit })
                if path == "bin/example" && limit == MAX_WRAPPER_BYTES
        ),
        "the size cap refuses it before the wrapper is read whole"
    );
}

#[test]
fn the_manifest_binary_must_be_the_wrapper() {
    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "manifest.json",
        file(
            manifest_json("example", "1.0.0")
                .replace("bin/example", "bin/other")
                .as_bytes(),
        ),
    );
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::BinaryPath { declared, expected })
            if declared == "bin/other" && expected == "bin/example"
    ));
}

#[test]
fn a_version_longer_than_the_manifest_cap_is_refused() {
    let at_cap = format!("1.0.0-{}", "x".repeat(64 - "1.0.0-".len()));
    assert!(validate_dir(widget_dir("example", &at_cap)).is_ok());

    assert!(matches!(
        validate_dir(widget_dir("example", &format!("{at_cap}x"))),
        Err(WidgetPackageError::Manifest(
            ManifestError::VersionTooLong { max: 64 }
        ))
    ));
}

#[test]
fn a_declared_icon_must_exist_under_assets() {
    let mut widget = widget_dir("example", "1.0.0");
    remove(&mut widget, "assets/icon.svg");
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Icon(path)) if path == "assets/icon.svg"
    ));
}

#[test]
fn the_icon_path_may_not_leave_assets_through_the_filesystem() {
    let mut widget = widget_dir("example", "1.0.0");
    let manifest = manifest_json("example", "1.0.0").replace(
        "\"icon\": \"assets/icon.svg\"",
        "\"icon\": \"assets/../manifest.json\"",
    );
    assert_ne!(
        manifest,
        manifest_json("example", "1.0.0"),
        "the fixture declares an icon"
    );
    set(&mut widget, "manifest.json", file(manifest.as_bytes()));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Icon(path)) if path == "assets/../manifest.json"
    ));
}

#[test]
fn an_icon_that_is_not_an_image_is_refused() {
    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "assets/icon.html",
        file(b"<script>fetch('/api')</script>"),
    );
    let manifest = manifest_json("example", "1.0.0").replace(
        "\"icon\": \"assets/icon.svg\"",
        "\"icon\": \"assets/icon.html\"",
    );
    set(&mut widget, "manifest.json", file(manifest.as_bytes()));
    assert!(
        matches!(
            validate_dir(widget),
            Err(WidgetPackageError::Icon(path)) if path == "assets/icon.html"
        ),
        "the icon route serves the file typed by its extension, on the device's origin"
    );
}

#[test]
fn an_icon_extension_is_matched_regardless_of_case() {
    let mut widget = widget_dir("example", "1.0.0");
    remove(&mut widget, "assets/icon.svg");
    set(&mut widget, "assets/icon.PNG", file(b"\x89PNG"));
    let manifest = manifest_json("example", "1.0.0").replace(
        "\"icon\": \"assets/icon.svg\"",
        "\"icon\": \"assets/icon.PNG\"",
    );
    set(&mut widget, "manifest.json", file(manifest.as_bytes()));
    validate_dir(widget).expect("BUG: an upper-case image extension is still an image");
}

#[test]
fn a_tree_nested_past_the_depth_cap_is_refused_without_walking_it() {
    let mut widget = widget_dir("example", "1.0.0");
    let mut nested = file(b"x");
    for _ in 0..=MAX_DEPTH {
        nested = dir(vec![("d", nested)]);
    }
    set(&mut widget, "assets/deep", nested);
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::TooDeep(path)) if path.starts_with("assets/deep/")
    ));
}

#[test]
fn a_missing_wasm_module_or_a_non_wasm_one_is_refused() {
    let mut widget = widget_dir("example", "1.0.0");
    remove(&mut widget, "lib/wasm/example.wasm");
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::Missing(path)) if path == "lib/wasm/example.wasm"
    ));

    let mut widget = widget_dir("example", "1.0.0");
    set(&mut widget, "lib/wasm/example.wasm", file(b"\x7fELF"));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::NotWasm(path)) if path == "lib/wasm/example.wasm"
    ));
}

#[test]
fn only_the_wrapper_may_be_executable() {
    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "bin/example",
        file(expected_wrapper("example", SDK_MAJOR).as_bytes()),
    );
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::WrapperNotExecutable(_))
    ));

    let mut widget = widget_dir("example", "1.0.0");
    set(&mut widget, "assets/run.sh", executable(b"#!/bin/sh\n"));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::UnexpectedExecutable(path)) if path == "assets/run.sh"
    ));
}

#[test]
fn the_expected_wrapper_is_the_nix_template_with_the_launcher_filled_in() {
    let wrapper = expected_wrapper("example", SDK_MAJOR);
    assert!(
        !wrapper.contains("@sdkMajor@") && !wrapper.contains("@name@"),
        "every placeholder is filled: {wrapper}"
    );
    assert!(wrapper.contains(&format!(
        "/run/current-profile/bin/bmc-wasm-thin-v{SDK_MAJOR} "
    )));
    assert!(wrapper.contains("\"${0%/*}/../lib/wasm/example.wasm\""));
}

#[test]
fn a_wrapper_naming_this_major_another_way_is_not_an_sdk_mismatch() {
    let mut dir = widget_dir("example", "1.0.0");
    let wrapper = WRAPPER_TEMPLATE
        .replace("@sdkMajor@", &format!("0{SDK_MAJOR}"))
        .replace("@name@", "example");
    set(&mut dir, "bin/example", executable(wrapper.as_bytes()));
    assert!(
        matches!(
            validate_dir(dir),
            Err(WidgetPackageError::Wrapper(path)) if path == "bin/example"
        ),
        "\"built for SDK vN, this device runs SDK vN\" would contradict itself"
    );
}

#[test]
fn a_wrapper_with_a_second_command_is_refused() {
    let mut dir = widget_dir("example", "1.0.0");
    let two_commands = format!(
        "#!/bin/sh\nrm -rf /\nexec /run/current-profile/bin/bmc-wasm-thin-v{SDK_MAJOR} --wasm \"${{0%/*}}/../lib/wasm/example.wasm\" --asset-root \"${{0%/*}}/../lib/assets\" \"$@\"\n"
    );
    set(&mut dir, "bin/example", executable(two_commands.as_bytes()));
    assert!(
        matches!(
            validate_dir(dir),
            Err(WidgetPackageError::Wrapper(path)) if path == "bin/example"
        ),
        "a line without a continuation is another command, not more of this one"
    );
}

#[test]
fn a_directory_with_too_many_entries_is_refused_without_listing_it() {
    let mut widget = widget_dir("example", "1.0.0");
    let flood = (0..=MAX_DIR_ENTRIES)
        .map(|i| (format!("f{i}"), file(b"")))
        .collect::<BTreeMap<_, _>>();
    set(&mut widget, "assets/flood", Node::Directory(flood));
    assert!(matches!(
        validate_dir(widget),
        Err(WidgetPackageError::TooManyEntries(path)) if path == "assets/flood"
    ));
}

#[tokio::test(start_paused = true)]
async fn an_upload_that_stalls_past_the_deadline_times_out() {
    use tokio::io::AsyncReadExt as _;

    let (stalled, _client_still_connected) = tokio::io::duplex(1);
    let mut upload = deadline_archive(Box::pin(stalled));
    let mut sink = Vec::new();
    let error = upload
        .read_to_end(&mut sink)
        .await
        .expect_err("BUG: a reader that never finishes must hit the deadline");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn a_wrapper_split_on_what_sh_does_not_split_on_is_refused() {
    for separator in ['\u{a0}', '\r', '\u{c}'] {
        let mut dir = widget_dir("example", "1.0.0");
        set(
            &mut dir,
            "bin/example",
            executable(
                format!(
                    "#!/bin/sh\nexec{separator}/run/current-profile/bin/bmc-wasm-thin-v{SDK_MAJOR} --wasm \"${{0%/*}}/../lib/wasm/example.wasm\" --asset-root \"${{0%/*}}/../lib/assets\" \"$@\"\n"
                )
                .as_bytes(),
            ),
        );
        assert!(
            matches!(
                validate_dir(dir),
                Err(WidgetPackageError::Wrapper(path)) if path == "bin/example"
            ),
            "sh splits words on space and tab only; {separator:?} would leave the command not found"
        );
    }
}

#[test]
fn a_reason_quotes_package_values_only_as_far_as_a_toast_shows() {
    let declared = format!("bin/{}", "x".repeat(4096));
    let mut widget = widget_dir("example", "1.0.0");
    set(
        &mut widget,
        "manifest.json",
        file(
            manifest_json("example", "1.0.0")
                .replace("bin/example", &declared)
                .as_bytes(),
        ),
    );
    let reason = validate_dir(widget)
        .expect_err("BUG: the declared binary is not the wrapper")
        .to_string();
    assert!(
        !reason.contains(&declared) && reason.chars().count() < 200,
        "the uploader sees the reason in a toast, not a manifest dump: {reason}"
    );

    let names: Vec<String> = (0..200).map(|index| format!("widget-{index}")).collect();
    let widgets = names
        .iter()
        .map(|name| (name.as_str(), Node::Directory(widget_dir(name, "1.0.0"))))
        .collect();
    let tree = dir(vec![("lib", dir(vec![("bmc-widgets", dir(widgets))]))]);
    let reason = validate(&tree)
        .expect_err("BUG: two hundred widgets are not one")
        .to_string();
    assert!(
        reason.chars().count() < 200,
        "a directory listing is cut, not sent whole: {reason}"
    );
}

#[test]
fn a_value_is_cut_on_a_character_not_a_byte() {
    let fits = "é".repeat(MAX_SHOWN_CHARS);
    assert_eq!(shown(&fits), fits, "a value at the cap is quoted whole");
    assert_eq!(
        shown(&format!("{fits}é")),
        format!("{fits}…"),
        "a multi-byte value one past the cap must cut cleanly, not panic"
    );
}

#[test]
fn a_wrapper_mismatch_is_reported_in_one_line() {
    let mut dir = widget_dir("example", "1.0.0");
    set(&mut dir, "bin/example", executable(b"#!/bin/sh\n"));
    let reason = validate_dir(dir)
        .expect_err("BUG: an empty wrapper is not the template")
        .to_string();
    assert!(
        !reason.contains('\n'),
        "the reason names the file; the template is the SDK's to show: {reason}"
    );
}

#[test]
fn a_name_that_is_not_utf8_is_the_packages_fault() {
    use std::os::unix::ffi::OsStrExt as _;

    let staging = tempfile::tempdir().expect("BUG: tempdir");
    let root = staging.path().join("package");
    restore(
        &encode(&package_tree("example", widget_dir("example", "1.2.3"))),
        &root,
        CaseHack::Disabled,
    )
    .expect("BUG: fixture restores");
    std::fs::write(root.join(std::ffi::OsStr::from_bytes(b"bad-\xff")), b"")
        .expect("BUG: the staging fs takes any bytes");

    let error = WidgetPackage::from_dir(&root, SDK_MAJOR).expect_err("BUG: the name is not UTF-8");

    assert!(
        matches!(&error, WidgetPackageError::NotUtf8Name(name) if name == "bad-\u{fffd}"),
        "{error}"
    );
    assert!(
        matches!(
            crate::packages::package_error(error),
            crate::packages::WidgetInstallError::Rejected(_)
        ),
        "the uploader sent the name, so it is a refusal, not a device fault"
    );
}

/// A widgets directory with each folder holding a `manifest.json` of the given contents.
fn installed_widgets(widgets: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    for (folder, manifest) in widgets {
        let widget = dir.path().join(folder);
        std::fs::create_dir_all(&widget).expect("BUG: widget dir");
        std::fs::write(widget.join("manifest.json"), manifest).expect("BUG: write manifest");
    }
    dir
}

fn example_package() -> WidgetPackage {
    validate_dir(widget_dir("example", "1.0.0")).expect("BUG: valid fixture")
}

#[test]
fn a_uid_clash_is_found_as_deep_as_discovery_looks_and_no_deeper() {
    let manifest = manifest_json("clock", "1.0.0");
    let within = installed_widgets(&[("a/b/clock", &manifest)]);
    let beyond = installed_widgets(&[("a/b/c/clock", &manifest)]);

    assert_eq!(
        example_package().uid_clash(within.path()),
        Some(PathBuf::from("a/b/clock"))
    );
    assert_eq!(
        example_package().uid_clash(beyond.path()),
        None,
        "discovery never loads a widget four folders down, so it hides nothing"
    );
}

#[test]
fn a_folder_below_an_installed_widget_is_not_a_widget() {
    let other =
        manifest_json("other", "1.0.0").replace(UID, "6ba7b810-9dad-41d1-80b4-00c04fd430c8");
    let widgets = installed_widgets(&[
        ("other", &other),
        ("other/clock", &manifest_json("clock", "1.0.0")),
    ]);

    assert_eq!(
        example_package().uid_clash(widgets.path()),
        None,
        "discovery stops at the first manifest on a path"
    );
}

#[test]
fn an_installed_manifest_that_does_not_parse_holds_no_uid() {
    let widgets = installed_widgets(&[("broken", &format!(r#"{{"uid": "{UID}""#))]);

    assert_eq!(
        example_package().uid_clash(widgets.path()),
        None,
        "the registry skips a widget it cannot parse, so it hides nothing"
    );
}

#[test]
fn a_widget_folder_the_profile_links_to_is_searched() {
    let store = installed_widgets(&[("clock", &manifest_json("clock", "1.0.0"))]);
    let widgets = tempfile::tempdir().expect("BUG: tempdir");
    std::os::unix::fs::symlink(store.path().join("clock"), widgets.path().join("clock"))
        .expect("BUG: link widget");

    assert_eq!(
        example_package().uid_clash(widgets.path()),
        Some(PathBuf::from("clock"))
    );
}
