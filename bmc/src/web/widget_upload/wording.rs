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

//! What a refused widget package upload reads as.
//! Device paths, nix output and the OS's own text stay in the log.

use bmc_intl::{Intl, format_message};
use bmc_upgrade::packages::WidgetInstallError;
use bmc_upgrade::widget_package::{
    FolderClash, MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_MIB, MAX_DEPTH, MAX_DIR_ENTRIES,
    MAX_WIDGET_NAME_LEN, UPLOAD_DEADLINE_MINS, WidgetPackageError, shown,
};

fn count(limit: usize) -> u64 {
    u64::try_from(limit).expect("BUG: a package limit fits in u64")
}

pub(super) fn wrong_content_type(intl: &Intl, content_type: &str) -> String {
    format_message!(
        intl,
        default_message: "The archive must be sent as {content_type}.",
        description: "Error when a widget package upload does not say it is an archive; content_type is a technical name",
        values: { content_type: content_type },
    )
}

pub(super) fn install_failure(intl: &Intl, error: &WidgetInstallError) -> String {
    match error {
        WidgetInstallError::Rejected(rejected) => package_rejected(intl, rejected),
        WidgetInstallError::Downgrade {
            package_name,
            installed,
            uploaded,
        } => format_message!(
            intl,
            default_message: "{package_name} {installed} is installed; uploading the older {uploaded} would downgrade it.",
            description: "Error when uploading a widget older than the installed one; package_name is its name; installed and uploaded are versions",
            values: {
                package_name: shown(package_name),
                installed: shown(installed),
                uploaded: shown(uploaded),
            },
        ),
        WidgetInstallError::FolderTaken {
            uploaded,
            installed: FolderClash::FlatWidget(name),
        } => format_message!(
            intl,
            default_message: "{uploaded} cannot be installed next to the installed widget {name}: only one of them would show.",
            description: "Error when an uploaded widget's folder clashes with an installed widget; uploaded and name are folder names",
            values: { uploaded: uploaded.display().to_string(), name: name.as_str() },
        ),
        WidgetInstallError::FolderTaken {
            uploaded,
            installed: FolderClash::MaintainerFolder(name),
        } => format_message!(
            intl,
            default_message: "{uploaded} cannot be installed next to the widgets installed under {name}/: only one of them would show.",
            description: "Error when an uploaded widget's folder clashes with a folder of installed widgets; uploaded and name are folder names",
            values: { uploaded: uploaded.display().to_string(), name: name.as_str() },
        ),
        WidgetInstallError::UidTaken {
            uploaded,
            installed,
        } => format_message!(
            intl,
            default_message: "{uploaded} has the uid of the installed widget {installed}: only one of them would show.",
            description: "Error when an uploaded widget has the identifier of an installed one; uploaded and installed are folder names; uid is short for unique identifier",
            values: {
                uploaded: uploaded.display().to_string(),
                installed: installed.display().to_string(),
            },
        ),
        WidgetInstallError::UploadStalled => format_message!(
            intl,
            default_message: "{minutes, plural, one {Upload did not finish within {minutes} minute!} other {Upload did not finish within {minutes} minutes!}}",
            description: "Error when a widget package upload took too long; minutes is how many minutes it may take",
            values: { minutes: UPLOAD_DEADLINE_MINS },
        ),
        WidgetInstallError::UploadAborted => format_message!(
            intl,
            default_message: "The upload ended before the archive was complete.",
            description: "Error when a widget package upload broke off midway",
        ),
        WidgetInstallError::ProfileBusy => format_message!(
            intl,
            default_message: "Another install or upgrade is in progress.",
            description: "Error when uploading a widget package while the device is changing its software",
        ),
        WidgetInstallError::Install(bmc_nix::upgrade::InstallError::NotEnoughSpace { .. }) => {
            format_message!(
                intl,
                default_message: "Not enough space on the device to install the widget!",
                description: "Error when the device has too little free storage for an uploaded widget",
            )
        }
        WidgetInstallError::ReadProfile(_)
        | WidgetInstallError::Staging(_)
        | WidgetInstallError::Store(_)
        | WidgetInstallError::Install(_)
        | WidgetInstallError::Failed(_) => format_message!(
            intl,
            default_message: "Widget install failed. See the device log for details.",
            description: "Error when the device could not install an uploaded widget for a reason of its own",
        ),
    }
}

/// Package-controlled values are cut to what a toast has room for.
#[expect(
    clippy::too_many_lines,
    reason = "one sentence per way a package can be refused"
)]
pub(super) fn package_rejected(intl: &Intl, error: &WidgetPackageError) -> String {
    match error {
        WidgetPackageError::TooLarge => format_message!(
            intl,
            default_message: "Widget package rejected: archive is larger than the {limit} MiB limit.",
            description: "Error when an uploaded widget package is too big; limit is a whole number",
            values: { limit: MAX_ARCHIVE_MIB.to_string() },
        ),
        WidgetPackageError::Archive(_) => format_message!(
            intl,
            default_message: "Widget package rejected: not a valid NAR archive.",
            description: "Error when an uploaded file cannot be read as an archive; NAR is a file format",
        ),
        WidgetPackageError::TooManyArchiveEntries => format_message!(
            intl,
            default_message: "{limit, plural, one {Widget package rejected: archive holds more than {limit} file or folder.} other {Widget package rejected: archive holds more than {limit} files and folders.}}",
            description: "Error when an uploaded widget package has too many files; limit is the most files and folders it may hold",
            values: { limit: count(MAX_ARCHIVE_ENTRIES) },
        ),
        WidgetPackageError::Io { path, .. } => format_message!(
            intl,
            default_message: "Widget package rejected: cannot read \"{path}\".",
            description: "Error when a file in an uploaded widget package cannot be read; path is its path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::FileTooLarge { path, limit } => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is larger than the {limit}-byte limit.",
            description: "Error when a file in an uploaded widget package is too big; path is its path in the package; limit is a whole number",
            values: { path: shown(path), limit: limit.to_string() },
        ),
        WidgetPackageError::NotOneWidget(found) if found.is_empty() => format_message!(
            intl,
            default_message: "Widget package rejected: archive must contain exactly one widget folder (holding manifest.json) under lib/bmc-widgets/, found none.",
            description: "Error on an uploaded widget package with no widget in it; manifest.json and lib/bmc-widgets/ stay as they are",
        ),
        WidgetPackageError::NotOneWidget(found) => format_message!(
            intl,
            default_message: "Widget package rejected: archive must contain exactly one widget folder (holding manifest.json) under lib/bmc-widgets/, found: {found}.",
            description: "Error on an uploaded widget package without one widget in it; found lists the folders it does have; manifest.json and lib/bmc-widgets/ stay as they are",
            values: { found: shown(found) },
        ),
        WidgetPackageError::NotADirectory(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is not a directory.",
            description: "Error on an uploaded widget package; path is a path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::BadWidgetName(name) => format_message!(
            intl,
            default_message: "{limit, plural, one {Widget package rejected: directory name \"{name}\" must be at most {limit} lowercase letter, digit or dash, not starting with a dash.} other {Widget package rejected: directory name \"{name}\" must be at most {limit} lowercase letters, digits and dashes, not starting with a dash.}}",
            description: "Error on an uploaded widget package whose folder name is not allowed; name is that name; limit is the most characters it may have",
            values: { name: shown(name), limit: count(MAX_WIDGET_NAME_LEN) },
        ),
        WidgetPackageError::NotUtf8Name(name) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{name}\" is not a UTF-8 name.",
            description: "Error on an uploaded widget package holding a file name in the wrong encoding; name is that name; UTF-8 is an encoding",
            values: { name: shown(name) },
        ),
        WidgetPackageError::Missing(path) => format_message!(
            intl,
            default_message: "Widget package rejected: missing \"{path}\".",
            description: "Error on an uploaded widget package lacking a required file; path is that file's path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::NativeBinary(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is a native widget binary; only WASM widgets can be installed.",
            description: "Error on an uploaded widget package with a program built for the device; path is its path in the package; WASM is a technology name",
            values: { path: shown(path) },
        ),
        WidgetPackageError::Symlink(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is a symlink; the package may not contain symlinks.",
            description: "Error on an uploaded widget package with a symbolic link in it; path is its path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::TooDeep(path) => format_message!(
            intl,
            default_message: "{limit, plural, one {Widget package rejected: \"{path}\" is nested deeper than {limit} directory.} other {Widget package rejected: \"{path}\" is nested deeper than {limit} directories.}}",
            description: "Error on an uploaded widget package with folders too deep; path is a path in the package; limit is the most levels of folders allowed",
            values: { path: shown(path), limit: count(MAX_DEPTH) },
        ),
        WidgetPackageError::TooManyEntries(path) => format_message!(
            intl,
            default_message: "{limit, plural, one {Widget package rejected: \"{path}\" holds more than {limit} entry.} other {Widget package rejected: \"{path}\" holds more than {limit} entries.}}",
            description: "Error on an uploaded widget package with too many files in one folder; path is that folder's path in the package; limit is the most entries a folder may hold",
            values: { path: shown(path), limit: count(MAX_DIR_ENTRIES) },
        ),
        WidgetPackageError::Unexpected(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is not part of a WASM widget package.",
            description: "Error on an uploaded widget package with a file that does not belong; path is its path in the package; WASM is a technology name",
            values: { path: shown(path) },
        ),
        WidgetPackageError::UnexpectedExecutable(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" must not be executable.",
            description: "Error on an uploaded widget package with a file wrongly marked as a program; path is its path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::Manifest(manifest) => format_message!(
            intl,
            default_message: "Widget package rejected: manifest.json is invalid: {detail}.",
            description: "Error on an uploaded widget package with a broken manifest; detail is an untranslated technical explanation for the widget's author; manifest.json stays as it is",
            values: { detail: shown(&manifest.to_string()) },
        ),
        WidgetPackageError::BinaryPath { declared, expected } => format_message!(
            intl,
            default_message: "Widget package rejected: manifest.json declares binary \"{declared}\", expected \"{expected}\".",
            description: "Error on an uploaded widget package naming the wrong program; declared and expected are paths in the package; manifest.json stays as it is",
            values: { declared: shown(declared), expected: shown(expected) },
        ),
        WidgetPackageError::Programs => format_message!(
            intl,
            default_message: "Widget package rejected: manifest.json declares programs, which an upload cannot ship yet.",
            description: "Error on an uploaded widget package whose manifest lists extra programs; manifest.json stays as it is",
        ),
        WidgetPackageError::Icon(path) => format_message!(
            intl,
            default_message: "Widget package rejected: manifest.json icon \"{path}\" must be an svg, png, webp, jpg or jpeg file under assets/.",
            description: "Error on an uploaded widget package with an unusable icon; path is the icon's path in the package; manifest.json and assets/ stay as they are",
            values: { path: shown(path) },
        ),
        WidgetPackageError::WrapperNotExecutable(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" must be executable.",
            description: "Error on an uploaded widget package with a launcher not marked as a program; path is its path in the package",
            values: { path: shown(path) },
        ),
        WidgetPackageError::ProtocolMajor { package, device } => format_message!(
            intl,
            default_message: "Widget package rejected: widget was built with WASM protocol v{package}, but this device supports only v{device}. Rebuild it with a matching SDK, or update the device if the widget's protocol is newer.",
            description: "Error on an uploaded widget package built for another version of the device's interface; package and device are whole version numbers; WASM and SDK are technology names",
            values: { package: package.to_string(), device: device.to_string() },
        ),
        WidgetPackageError::Wrapper(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is not the launcher wrapper the widget SDK generates.",
            description: "Error on an uploaded widget package with a launcher someone changed; path is its path in the package; SDK is a technology name",
            values: { path: shown(path) },
        ),
        WidgetPackageError::NotWasm(path) => format_message!(
            intl,
            default_message: "Widget package rejected: \"{path}\" is not a WASM module.",
            description: "Error on an uploaded widget package whose program is of the wrong kind; path is its path in the package; WASM is a technology name",
            values: { path: shown(path) },
        ),
    }
}
