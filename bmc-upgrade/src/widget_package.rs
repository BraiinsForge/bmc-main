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

//! A WASM widget package uploaded as a NAR, checked against the layout
//! `nix/wasm-widgets.nix` produces before anything reaches the store.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmc_nix::types::{InstalledBy, ResolvedPackage};
use bmc_widget_manifest::{MAX_WIDGET_DIR_DEPTH, Manifest, ManifestError};

/// Upper bound on an uploaded archive. Built widgets are a few hundred
/// kilobytes, an asset-heavy one tens of megabytes;
/// the bound keeps a hostile upload from tying up the device.
/// The archive is saved whole before it is unpacked, so the bound is on
/// disk and time.
pub const MAX_ARCHIVE_MIB: u64 = 64;
pub const MAX_ARCHIVE_BYTES: u64 = MAX_ARCHIVE_MIB * 1024 * 1024;

/// Files, directories and symlinks an archive may hold in all. They are
/// counted before any is unpacked: a flood of one-byte files costs
/// a 4 KiB block and an inode each, far more than its share of the archive.
pub const MAX_ARCHIVE_ENTRIES: usize = 4096;

/// Total time an upload may take, slow link or not; a client that trickles
/// bytes would otherwise hold the upgrade gate forever.
pub const UPLOAD_DEADLINE_MINS: u64 = 5;
pub const UPLOAD_DEADLINE: Duration = Duration::from_secs(UPLOAD_DEADLINE_MINS * 60);

/// The SDK major the device's launcher serves; a wrapper for another one
/// would exec a launcher that is not in the profile.
pub const WASM_SDK_MAJOR: u16 = bmc_wasm_protocol::SDK_VERSION.0;

/// Package-name prefix shared with `nix/packages.nix`.
pub const PACKAGE_NAME_PREFIX: &str = "widget-";

const WASM_MAGIC: &[u8] = b"\0asm\x01\0\0\0";
const ELF_MAGIC: &[u8] = b"\x7fELF";
const ICON_EXTENSIONS: [&str; 5] = ["svg", "png", "webp", "jpg", "jpeg"];
/// Caps for the two files validation reads whole; the rest is checked by
/// prefix. Keeps a near-limit archive from costing a near-limit allocation.
const MAX_WRAPPER_BYTES: usize = 4 * 1024;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
/// Directory levels below the widget directory; the walk recurses per level.
const MAX_DEPTH: usize = 32;
/// Entries listed per directory before giving up; an archive of a million
/// empty files is not a widget and must not cost a million-name allocation.
const MAX_DIR_ENTRIES: usize = 4096;
/// Characters of a package-controlled value a reason quotes; a toast has no
/// room for a manifest field or directory listing of any length.
const MAX_SHOWN_CHARS: usize = 80;
/// The wrapper `nix/wasm-widgets.nix` installs for a profile-mode widget,
/// shared with the build so the check cannot drift from what it produces.
const WRAPPER_TEMPLATE: &str = include_str!("../../nix/wasm-widget-wrapper.sh");

/// `value`, cut to `MAX_SHOWN_CHARS` with an ellipsis when longer.
#[must_use]
pub fn shown(value: &str) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(MAX_SHOWN_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WidgetPackageError {
    #[error("archive is larger than the {MAX_ARCHIVE_MIB} MiB limit")]
    TooLarge,
    #[error("not a valid NAR archive: {}", shown(.0))]
    Archive(String),
    #[error("archive holds more than {MAX_ARCHIVE_ENTRIES} files and folders")]
    TooManyArchiveEntries,
    #[error("cannot read '{}': {source}", shown(.path))]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("'{}' is larger than the {limit}-byte limit", shown(.path))]
    FileTooLarge { path: String, limit: usize },
    #[error(
        "archive must contain exactly one widget folder (holding manifest.json) under lib/bmc-widgets/, found: {}",
        shown(.0)
    )]
    NotOneWidget(String),
    #[error("'{}' is not a directory", shown(.0))]
    NotADirectory(String),
    #[error(
        "directory name '{}' must be at most {MAX_WIDGET_NAME_LEN} lowercase letters, digits and dashes, not starting with a dash",
        shown(.0)
    )]
    BadWidgetName(String),
    #[error("'{}' is not a UTF-8 name", shown(.0))]
    NotUtf8Name(String),
    #[error("missing '{}'", shown(.0))]
    Missing(String),
    #[error("'{}' is a native widget binary; only WASM widgets can be installed", shown(.0))]
    NativeBinary(String),
    #[error("'{}' is a symlink; the package may not contain symlinks", shown(.0))]
    Symlink(String),
    #[error("'{}' is nested deeper than {MAX_DEPTH} directories", shown(.0))]
    TooDeep(String),
    #[error("'{}' holds more than {MAX_DIR_ENTRIES} entries", shown(.0))]
    TooManyEntries(String),
    #[error("'{}' is not part of a WASM widget package", shown(.0))]
    Unexpected(String),
    #[error("'{}' must not be executable", shown(.0))]
    UnexpectedExecutable(String),
    #[error("manifest.json is invalid: {}", shown(&.0.to_string()))]
    Manifest(#[source] ManifestError),
    #[error("manifest.json declares binary '{}', expected '{}'", shown(.declared), shown(.expected))]
    BinaryPath { declared: String, expected: String },
    #[error(
        "manifest.json icon '{}' must be an svg, png, webp, jpg or jpeg file under assets/",
        shown(.0)
    )]
    Icon(String),
    #[error("'{}' must be executable", shown(.0))]
    WrapperNotExecutable(String),
    #[error("widget was built for SDK v{package}, this device runs SDK v{device}")]
    SdkMajor { package: u16, device: u16 },
    #[error("'{}' is not the launcher wrapper the widget SDK generates", shown(.0))]
    Wrapper(String),
    #[error("'{}' is not a WASM module", shown(.0))]
    NotWasm(String),
}

/// A validated upload: the widget's directory names and its manifest.
#[derive(Debug, Clone)]
pub struct WidgetPackage {
    maintainer: Option<String>,
    name: String,
    manifest: Manifest,
}

/// An installed folder an upload cannot sit next to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderClash {
    /// A flat widget named like the upload's maintainer folder.
    FlatWidget(String),
    /// A maintainer folder named like the flat upload.
    MaintainerFolder(String),
}

impl std::fmt::Display for FolderClash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FlatWidget(name) => write!(f, "the installed widget {name}"),
            Self::MaintainerFolder(name) => write!(f, "the widgets installed under {name}/"),
        }
    }
}

/// An uploaded archive as it arrives.
pub type ArchiveStream = std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>;

/// An archive and, when the client announced one, its size,
/// so an archive that will not fit is refused before a byte of it is staged.
pub struct WidgetUpload {
    pub nar: ArchiveStream,
    pub announced_bytes: Option<u64>,
}

impl std::fmt::Debug for WidgetUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetUpload")
            .field("announced_bytes", &self.announced_bytes)
            .finish_non_exhaustive()
    }
}

impl WidgetUpload {
    #[must_use]
    pub fn unannounced(nar: ArchiveStream) -> Self {
        Self {
            nar,
            announced_bytes: None,
        }
    }
}

/// Fail `nar` with [`std::io::ErrorKind::TimedOut`] once [`UPLOAD_DEADLINE`] has
/// passed since the wrapper was created, whether or not bytes still flow.
#[must_use]
pub fn deadline_archive(nar: ArchiveStream) -> ArchiveStream {
    Box::pin(DeadlineArchive {
        nar,
        deadline: Box::pin(tokio::time::sleep(UPLOAD_DEADLINE)),
    })
}

struct DeadlineArchive {
    nar: ArchiveStream,
    deadline: std::pin::Pin<Box<tokio::time::Sleep>>,
}

impl tokio::io::AsyncRead for DeadlineArchive {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.deadline.as_mut().poll(cx).is_ready() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("upload did not finish within {UPLOAD_DEADLINE_MINS} minutes"),
            )));
        }
        self.nar.as_mut().poll_read(cx, buf)
    }
}

/// Cap `nar` at [`MAX_ARCHIVE_BYTES`]: reading past the limit fails with
/// [`std::io::ErrorKind::FileTooLarge`], so whatever consumes the stream
/// stops without ever holding more than the limit.
#[must_use]
pub fn limit_archive(nar: ArchiveStream) -> ArchiveStream {
    Box::pin(LimitedArchive {
        nar,
        remaining: MAX_ARCHIVE_BYTES,
    })
}

struct LimitedArchive {
    nar: ArchiveStream,
    remaining: u64,
}

impl tokio::io::AsyncRead for LimitedArchive {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // Read into a window one byte wider than the budget: a read that
        // fills it proves the archive is over the limit, and the caller sees
        // none of those bytes.
        let budget = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let window = buf.remaining().min(budget.saturating_add(1));
        let mut probe = tokio::io::ReadBuf::new(buf.initialize_unfilled_to(window));
        std::task::ready!(self.nar.as_mut().poll_read(cx, &mut probe))?;
        let read = probe.filled().len();
        let Some(remaining) = u64::try_from(read)
            .ok()
            .and_then(|read| self.remaining.checked_sub(read))
        else {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                WidgetPackageError::TooLarge.to_string(),
            )));
        };
        self.remaining = remaining;
        buf.advance(read);
        std::task::Poll::Ready(Ok(()))
    }
}

impl WidgetPackage {
    /// Validate an unpacked archive rooted at `root` against the device's
    /// SDK major.
    ///
    /// # Errors
    ///
    /// The first rule the archive breaks, worded for the person who uploaded it.
    pub fn from_dir(root: &Path, sdk_major: u16) -> Result<Self, WidgetPackageError> {
        let WidgetDir {
            maintainer,
            name,
            path,
        } = single_widget_dir(root)?;
        let manifest = validate_widget_dir(&name, &path, sdk_major)?;
        Ok(Self {
            maintainer,
            name,
            manifest,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The widget's directory below `lib/bmc-widgets/`.
    #[must_use]
    pub fn relative_dir(&self) -> PathBuf {
        match &self.maintainer {
            Some(maintainer) => Path::new(maintainer).join(&self.name),
            None => PathBuf::from(&self.name),
        }
    }

    /// What installed under `widgets_dir` this widget would hide or be hidden by.
    /// Discovery stops at the first folder holding a manifest, so a flat `acme`
    /// and the widgets under a maintainer folder `acme/` cannot both show.
    #[must_use]
    pub fn folder_clash(&self, widgets_dir: &Path) -> Option<FolderClash> {
        if let Some(maintainer) = &self.maintainer {
            return has_manifest(&widgets_dir.join(maintainer))
                .then(|| FolderClash::FlatWidget(maintainer.clone()));
        }
        let folder = widgets_dir.join(&self.name);
        (folder.is_dir() && !has_manifest(&folder))
            .then(|| FolderClash::MaintainerFolder(self.name.clone()))
    }

    /// The installed folder under `widgets_dir`, other than this widget's own,
    /// whose manifest carries this widget's uid; the registry keys widgets
    /// by uid, so only one of the two would show. It walks as discovery
    /// walks, and skips a manifest it cannot read as discovery skips it.
    #[must_use]
    pub fn uid_clash(&self, widgets_dir: &Path) -> Option<PathBuf> {
        let own = self.relative_dir();
        let mut level = vec![PathBuf::new()];
        for _ in 0..MAX_WIDGET_DIR_DEPTH {
            let mut next = Vec::new();
            for parent in level {
                let Ok(entries) = std::fs::read_dir(widgets_dir.join(&parent)) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let folder = parent.join(entry.file_name());
                    let path = widgets_dir.join(&folder);
                    if !path.is_dir() {
                        continue;
                    }
                    if !path.join("manifest.json").exists() {
                        next.push(folder);
                    } else if folder != own
                        && installed_manifest(&path)
                            .is_some_and(|installed| installed.uid == self.manifest.uid)
                    {
                        return Some(folder);
                    }
                }
            }
            level = next;
        }
        None
    }

    /// `widget-<name>`, the package name the index would use for this widget,
    /// or `widget-<maintainer>_<name>` for a widget under a maintainer folder;
    /// neither name can contain `_`, so no two folders share a package.
    #[must_use]
    pub fn package_name(&self) -> String {
        match &self.maintainer {
            Some(maintainer) => format!("{PACKAGE_NAME_PREFIX}{maintainer}_{}", self.name),
            None => format!("{PACKAGE_NAME_PREFIX}{}", self.name),
        }
    }

    /// The profile entry for this package once it sits at `store_path`.
    #[must_use]
    pub fn resolved(&self, store_path: String) -> ResolvedPackage {
        ResolvedPackage {
            name: self.package_name(),
            version: self.manifest.version.to_string(),
            store_path,
            category: Some("widget".to_owned()),
            description: Some(self.manifest.description.clone()),
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::User,
            installed_from: "local".to_owned(),
            pinned: None,
            metadata: BTreeMap::new(),
        }
    }
}

fn io_error(path: &str, source: std::io::Error) -> WidgetPackageError {
    WidgetPackageError::Io {
        path: path.to_owned(),
        source,
    }
}

struct WidgetDir {
    maintainer: Option<String>,
    name: String,
    path: PathBuf,
}

/// `lib/bmc-widgets/<name>/` or `lib/bmc-widgets/<maintainer>/<name>/`
/// below `root`, the only thing a widget package may contain. A folder
/// without a manifest is a maintainer folder once a folder below it has one.
fn single_widget_dir(root: &Path) -> Result<WidgetDir, WidgetPackageError> {
    let mut dir = directory(root, "")?;
    let mut rel = String::new();
    for segment in ["lib", "bmc-widgets"] {
        let (name, path) = only_entry(&dir, &rel, Some(segment))?;
        rel = format!("{rel}{name}/");
        dir = directory(&path, &rel)?;
    }
    let (name, path) = only_entry(&dir, &rel, None)?;
    let path = directory(&path, &format!("{rel}{name}"))?;
    if !is_widget_name(&name) {
        return Err(WidgetPackageError::BadWidgetName(name));
    }
    if has_manifest(&path) {
        return Ok(WidgetDir {
            maintainer: None,
            name,
            path,
        });
    }

    let maintainer = name;
    let rel = format!("{rel}{maintainer}/");
    let names = entry_names(&path, &rel)?;
    let [name] = names.as_slice() else {
        let holds_a_widget = names.iter().any(|name| has_manifest(&path.join(name)));
        if !holds_a_widget {
            return Err(WidgetPackageError::Missing("manifest.json".to_owned()));
        }
        let found: Vec<_> = names
            .iter()
            .map(|name| format!("{maintainer}/{name}"))
            .collect();
        return Err(WidgetPackageError::NotOneWidget(found.join(", ")));
    };
    let name = name.clone();
    let path = directory(&path.join(&name), &format!("{rel}{name}"))?;
    if !is_widget_name(&name) {
        return Err(WidgetPackageError::BadWidgetName(name));
    }
    Ok(WidgetDir {
        maintainer: Some(maintainer),
        name,
        path,
    })
}

/// Any entry called `manifest.json`, so a symlink or a directory by that name
/// is refused as such by the widget checks, not read as a maintainer folder.
fn has_manifest(dir: &Path) -> bool {
    dir.join("manifest.json").symlink_metadata().is_ok()
}

/// Read as the registry reads it: an installed manifest is not size-capped.
fn installed_manifest(dir: &Path) -> Option<Manifest> {
    let contents = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    contents.parse().ok()
}

fn only_entry(
    dir: &Path,
    rel: &str,
    expected: Option<&str>,
) -> Result<(String, PathBuf), WidgetPackageError> {
    let names = entry_names(dir, rel)?;
    match names.as_slice() {
        [name] if expected.is_none_or(|expected| name == expected) => {
            Ok((name.clone(), dir.join(name)))
        }
        _ => Err(WidgetPackageError::NotOneWidget(names.join(", "))),
    }
}

fn entry_names(dir: &Path, rel: &str) -> Result<Vec<String>, WidgetPackageError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|source| io_error(rel, source))? {
        if names.len() == MAX_DIR_ENTRIES {
            return Err(WidgetPackageError::TooManyEntries(
                rel.trim_end_matches('/').to_owned(),
            ));
        }
        let entry = entry.map_err(|source| io_error(rel, source))?;
        let name = entry.file_name().into_string().map_err(|name| {
            WidgetPackageError::NotUtf8Name(format!("{rel}{}", name.to_string_lossy()))
        })?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// `path` if it is a directory, not a symlink to one.
fn directory(path: &Path, rel: &str) -> Result<PathBuf, WidgetPackageError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| io_error(rel, source))?;
    if metadata.is_dir() {
        Ok(path.to_path_buf())
    } else {
        Err(WidgetPackageError::NotADirectory(rel.to_owned()))
    }
}

/// Nix refuses store names over 211 bytes. Two names at this cap keep
/// `bmc-widget-<maintainer>_<widget>` well below it,
/// so a long name is a rejection the uploader sees, not an import failure.
const MAX_WIDGET_NAME_LEN: usize = 64;

fn is_widget_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_WIDGET_NAME_LEN
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn validate_widget_dir(
    name: &str,
    dir: &Path,
    sdk_major: u16,
) -> Result<Manifest, WidgetPackageError> {
    let wrapper_path = format!("bin/{name}");
    let wasm_path = format!("lib/wasm/{name}.wasm");

    let mut manifest_seen = false;
    let mut wasm_seen = false;
    let mut wrapper_executable = None;
    walk(dir, "", 0, &mut |path, executable| {
        if path == wrapper_path {
            wrapper_executable = Some(executable);
            return Ok(());
        }
        if path == "manifest.json" {
            manifest_seen = true;
        } else if path == wasm_path {
            wasm_seen = true;
        } else if !(path.starts_with("lib/assets/") || path.starts_with("assets/")) {
            return Err(WidgetPackageError::Unexpected(path.to_owned()));
        }
        if executable {
            return Err(WidgetPackageError::UnexpectedExecutable(path.to_owned()));
        }
        Ok(())
    })?;

    let Some(wrapper_executable) = wrapper_executable else {
        return Err(WidgetPackageError::Missing(wrapper_path));
    };
    let wrapper_script = read_capped(dir, &wrapper_path, MAX_WRAPPER_BYTES)?;
    if wrapper_script.starts_with(ELF_MAGIC) {
        return Err(WidgetPackageError::NativeBinary(wrapper_path));
    }

    if !manifest_seen {
        return Err(WidgetPackageError::Missing("manifest.json".to_owned()));
    }
    let manifest_json = read_capped(dir, "manifest.json", MAX_MANIFEST_BYTES)?;
    let manifest: Manifest = std::str::from_utf8(&manifest_json)
        .map_err(|_| {
            WidgetPackageError::Manifest(ManifestError::ParseError(serde::de::Error::custom(
                "not UTF-8",
            )))
        })?
        .parse()
        .map_err(WidgetPackageError::Manifest)?;
    if manifest.binary != Path::new(&wrapper_path) {
        return Err(WidgetPackageError::BinaryPath {
            declared: manifest.binary.display().to_string(),
            expected: wrapper_path,
        });
    }
    if let Some(icon) = &manifest.icon {
        let icon_path = icon.to_string_lossy();
        // The icon route types the file by its extension, so a non-image one
        // could render as a page. The list matches nix/public-asset.nix.
        let image = icon
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ICON_EXTENSIONS
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            });
        let present = icon_path.starts_with("assets/")
            && icon
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
            && std::fs::symlink_metadata(dir.join(icon)).is_ok_and(|metadata| metadata.is_file());
        if !(image && present) {
            return Err(WidgetPackageError::Icon(icon_path.into_owned()));
        }
    }

    if !wrapper_executable {
        return Err(WidgetPackageError::WrapperNotExecutable(wrapper_path));
    }
    check_wrapper(name, &wrapper_path, &wrapper_script, sdk_major)?;

    if !wasm_seen {
        return Err(WidgetPackageError::Missing(wasm_path));
    }
    if !read_prefix(dir, &wasm_path, WASM_MAGIC.len())?.starts_with(WASM_MAGIC) {
        return Err(WidgetPackageError::NotWasm(wasm_path));
    }
    Ok(manifest)
}

fn read_capped(dir: &Path, rel: &str, limit: usize) -> Result<Vec<u8>, WidgetPackageError> {
    let contents = read_prefix(dir, rel, limit + 1)?;
    if contents.len() > limit {
        return Err(WidgetPackageError::FileTooLarge {
            path: rel.to_owned(),
            limit,
        });
    }
    Ok(contents)
}

fn read_prefix(dir: &Path, rel: &str, len: usize) -> Result<Vec<u8>, WidgetPackageError> {
    use std::io::Read as _;

    let mut prefix = Vec::with_capacity(len);
    std::fs::File::open(dir.join(rel))
        .and_then(|file| {
            file.take(u64::try_from(len).unwrap_or(u64::MAX))
                .read_to_end(&mut prefix)
        })
        .map_err(|source| io_error(rel, source))?;
    Ok(prefix)
}

/// Hand every regular file below `dir` to `check` in path order,
/// one at a time: a tree of many files costs no memory per file.
/// Symlinks fail here so nothing in the package can point outside it.
fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    check: &mut impl FnMut(&str, bool) -> Result<(), WidgetPackageError>,
) -> Result<(), WidgetPackageError> {
    use std::os::unix::fs::PermissionsExt as _;

    for name in entry_names(dir, prefix)? {
        let path = format!("{prefix}{name}");
        let metadata =
            std::fs::symlink_metadata(dir.join(&name)).map_err(|source| io_error(&path, source))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(WidgetPackageError::Symlink(path));
        }
        if file_type.is_dir() {
            if depth == MAX_DEPTH {
                return Err(WidgetPackageError::TooDeep(path));
            }
            walk(&dir.join(&name), &format!("{path}/"), depth + 1, check)?;
        } else if file_type.is_file() {
            check(&path, metadata.permissions().mode() & 0o100 != 0)?;
        } else {
            return Err(WidgetPackageError::Unexpected(path));
        }
    }
    Ok(())
}

/// The wrapper `nix/wasm-widgets.nix` writes for a profile-mode widget;
/// the archive's wrapper must match it byte for byte.
#[must_use]
pub fn expected_wrapper(name: &str, sdk_major: u16) -> String {
    WRAPPER_TEMPLATE
        .replace("@sdkMajor@", &sdk_major.to_string())
        .replace("@name@", name)
}

fn check_wrapper(
    name: &str,
    path: &str,
    contents: &[u8],
    sdk_major: u16,
) -> Result<(), WidgetPackageError> {
    if contents == expected_wrapper(name, sdk_major).as_bytes() {
        return Ok(());
    }
    match wrapper_sdk_major(name, contents) {
        Some(package) if package != sdk_major => Err(WidgetPackageError::SdkMajor {
            package,
            device: sdk_major,
        }),
        _ => Err(WidgetPackageError::Wrapper(path.to_owned())),
    }
}

/// The SDK major of a wrapper that is the template in every other byte.
fn wrapper_sdk_major(name: &str, contents: &[u8]) -> Option<u16> {
    let template = WRAPPER_TEMPLATE.replace("@name@", name);
    let (before, after) = template
        .split_once("@sdkMajor@")
        .expect("BUG: the template names the launcher's SDK major");
    let major = contents
        .strip_prefix(before.as_bytes())?
        .strip_suffix(after.as_bytes())?;
    if major.is_empty() || !major.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(major).ok()?.parse().ok()
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
    //! Archive builders shared by the validator, backend and mock tests.

    use nix_archive::nar::{Encoder, Error};
    use std::collections::BTreeMap;

    pub const SDK_MAJOR: u16 = super::WASM_SDK_MAJOR;
    pub const UID: &str = "550e8400-e29b-41d4-a716-446655440000";

    /// An owned tree a test can edit before it is encoded.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Node {
        Regular { executable: bool, contents: Vec<u8> },
        Symlink { target: Vec<u8> },
        Directory(BTreeMap<String, Node>),
    }

    #[must_use]
    pub fn encode(tree: &Node) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new()).expect("BUG: a Vec accepts the NAR magic");
        encode_node(&mut encoder, None, tree).expect("BUG: a fixture tree encodes");
        encoder
            .finish()
            .expect("BUG: encode_node closes every directory")
    }

    fn encode_node(
        encoder: &mut Encoder<Vec<u8>>,
        name: Option<&[u8]>,
        node: &Node,
    ) -> Result<(), Error> {
        match node {
            Node::Regular {
                executable,
                contents,
            } => encoder.regular(name, *executable, contents),
            Node::Symlink { target } => encoder.symlink(name, target),
            Node::Directory(entries) => {
                encoder.start_directory(name)?;
                for (child, node) in entries {
                    encode_node(encoder, Some(child.as_bytes()), node)?;
                }
                encoder.end_directory()
            }
        }
    }

    #[must_use]
    pub fn file(contents: &[u8]) -> Node {
        Node::Regular {
            executable: false,
            contents: contents.to_vec(),
        }
    }

    #[must_use]
    pub fn executable(contents: &[u8]) -> Node {
        Node::Regular {
            executable: true,
            contents: contents.to_vec(),
        }
    }

    #[must_use]
    pub fn dir(entries: Vec<(&str, Node)>) -> Node {
        Node::Directory(
            entries
                .into_iter()
                .map(|(name, node)| (name.to_owned(), node))
                .collect(),
        )
    }

    #[must_use]
    pub fn manifest_json(name: &str, version: &str) -> String {
        format!(
            r#"{{
  "uid": "{UID}",
  "version": "{version}",
  "name": "Example",
  "description": "An example widget.",
  "binary": "bin/{name}",
  "icon": "assets/icon.svg",
  "supported_viewports": [{{"type": "rectangular", "min_width": 100, "max_width": 1280, "min_height": 100, "max_height": 480}}]
}}"#
        )
    }

    /// The widget directory `nix/wasm-widgets.nix` builds, as a mutable map
    /// so a test can break one thing.
    #[must_use]
    pub fn widget_dir(name: &str, version: &str) -> BTreeMap<String, Node> {
        let Node::Directory(entries) = dir(vec![
            (
                "manifest.json",
                file(manifest_json(name, version).as_bytes()),
            ),
            (
                "bin",
                dir(vec![(
                    name,
                    executable(super::expected_wrapper(name, SDK_MAJOR).as_bytes()),
                )]),
            ),
            (
                "lib",
                dir(vec![
                    (
                        "wasm",
                        dir(vec![(
                            &format!("{name}.wasm"),
                            file(b"\0asm\x01\0\0\0 module body"),
                        )]),
                    ),
                    (
                        "assets",
                        dir(vec![(
                            "v1",
                            dir(vec![("svg", dir(vec![("abc.asset", file(b"<svg/>"))]))]),
                        )]),
                    ),
                ]),
            ),
            ("assets", dir(vec![("icon.svg", file(b"<svg/>"))])),
        ]) else {
            unreachable!("BUG: dir() builds a directory")
        };
        entries
    }

    #[must_use]
    pub fn package_tree(name: &str, widget_dir: BTreeMap<String, Node>) -> Node {
        widgets_root(dir(vec![(name, Node::Directory(widget_dir))]))
    }

    /// The package tree with the widget under a maintainer folder.
    #[must_use]
    pub fn maintained_package_tree(
        maintainer: &str,
        name: &str,
        widget_dir: BTreeMap<String, Node>,
    ) -> Node {
        widgets_root(dir(vec![(
            maintainer,
            dir(vec![(name, Node::Directory(widget_dir))]),
        )]))
    }

    fn widgets_root(widgets: Node) -> Node {
        dir(vec![("lib", dir(vec![("bmc-widgets", widgets)]))])
    }

    #[must_use]
    pub fn valid_nar(name: &str, version: &str) -> Vec<u8> {
        encode(&package_tree(name, widget_dir(name, version)))
    }

    /// Reads as a body does once the browser tab closed mid-upload.
    #[derive(Debug)]
    pub struct ClientGone;

    impl tokio::io::AsyncRead for ClientGone {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "client went away",
            )))
        }
    }

    /// Replace the node at `path` (slash separated) inside `dir`.
    pub fn set(dir: &mut BTreeMap<String, Node>, path: &str, node: Node) {
        let (head, rest) = path.split_once('/').unwrap_or((path, ""));
        if rest.is_empty() {
            dir.insert(head.to_owned(), node);
            return;
        }
        let Some(Node::Directory(child)) = dir.get_mut(head) else {
            panic!("BUG: fixture path {path} crosses a non-directory");
        };
        set(child, rest, node);
    }

    pub fn remove(dir: &mut BTreeMap<String, Node>, path: &str) {
        let (head, rest) = path.split_once('/').unwrap_or((path, ""));
        if rest.is_empty() {
            dir.remove(head);
            return;
        }
        let Some(Node::Directory(child)) = dir.get_mut(head) else {
            panic!("BUG: fixture path {path} crosses a non-directory");
        };
        remove(child, rest);
    }
}

#[cfg(test)]
mod tests;
