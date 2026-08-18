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

mod filters;
mod format;

pub use filters::bmc::{
    BMC_CONFIG_DIR, BMC_CONFIG_LEGACY, BMC_SECRETS, BmcConfigCensor, SecretsExclusion,
    UciWirelessCensor,
};
pub use filters::{SupportFilter, censor};
pub use format::{ArchiveFormat, FinishWrite, PasswordProtectedZip, PlainZip};

use anyhow::Result;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use walkdir::WalkDir;
use zip::write::{SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, ZipWriter};

/// Generic Linux procfs paths worth capturing in a support archive.
pub const PROC_PATHS: &[&str] = &[
    "/proc/mounts",
    "/proc/loadavg",
    "/proc/cmdline",
    "/proc/crypto",
    "/proc/devices",
    "/proc/iomem",
    "/proc/ioports",
    "/proc/filesystems",
    "/proc/interrupts",
    "/proc/meminfo",
    "/proc/misc",
    "/proc/modules",
    "/proc/partitions",
    "/proc/stat",
    "/proc/uptime",
    "/proc/version",
    "/proc/net/arp",
];

const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";

/// Captured after every other diagnostic (see [`SupportConfig::collect`]) so
/// syslog that they trigger — e.g. dnsmasq reacting to the reachability
/// probe — is included in the snapshot.
const LOGREAD_COMMAND: &[&str] = &["logread"];

pub struct SupportConfig<'a> {
    commands: &'a [&'a [&'a str]],
    fs_paths: &'a [&'a str],
    ping_hosts: &'a [&'a str],
    filters: &'a [&'a dyn SupportFilter],
}

impl std::fmt::Debug for SupportConfig<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupportConfig")
            .field("commands", &self.commands)
            .field("fs_paths", &self.fs_paths)
            .field("ping_hosts", &self.ping_hosts)
            .field("filters", &self.filters.len())
            .finish()
    }
}

impl Default for SupportConfig<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SupportConfig<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: &[],
            fs_paths: &[],
            ping_hosts: &[],
            filters: &[],
        }
    }

    /// Commands whose stdout is captured. They run before the fs walk so any
    /// syslog they emit lands in the log file the walk collects.
    #[must_use]
    pub const fn commands(mut self, commands: &'a [&'a [&'a str]]) -> Self {
        self.commands = commands;
        self
    }

    /// Filesystem paths collected wholesale (files and directories).
    #[must_use]
    pub const fn fs_paths(mut self, fs_paths: &'a [&'a str]) -> Self {
        self.fs_paths = fs_paths;
        self
    }

    /// Hosts pinged for the reachability report.
    #[must_use]
    pub const fn ping_hosts(mut self, ping_hosts: &'a [&'a str]) -> Self {
        self.ping_hosts = ping_hosts;
        self
    }

    /// Credential filters applied to every collected file.
    #[must_use]
    pub const fn filters(mut self, filters: &'a [&'a dyn SupportFilter]) -> Self {
        self.filters = filters;
        self
    }

    /// Collect the support archive into `writer`, encoded per `format`.
    pub fn collect(
        &self,
        writer: impl Write,
        format: &dyn ArchiveFormat,
        compress: bool,
    ) -> Result<()> {
        let mut archive = SupportArchive::new(writer, format, compress, self.filters);

        // include outputs of commands
        // These run before the log capture below since some of them emit syslog (e.g. dnsmasq).
        for &cmdline in self.commands {
            match archive.add_cmd_output(cmdline) {
                Ok(()) => info!("Added output of '{}'", cmdline.join(" ")),
                Err(err) => error!("{}: '{}'", err, cmdline.join(" ")),
            }
        }

        // include output of builtin routines
        // Again these commands may produce some logs so log collection must be done after this.
        let ping_hosts = self.ping_hosts;
        let builtin_items: &[(&str, &dyn Fn() -> Option<String>)] = &[
            ("ifconfig", &|| Some(bmc_net_diag::ifconfig())),
            ("public_ip", &|| bmc_net_diag::public_ip().ok()),
            ("ping_report", &|| {
                bmc_net_diag::ping_report(ping_hosts).ok()
            }),
            ("timestamp", &|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs()
                    .to_string()
                    .into()
            }),
        ];
        for (name, function) in builtin_items {
            if let (name, Some(content)) = (name, function()) {
                match archive.add_builtin(name, &content) {
                    Ok(()) => info!("Added output of <{}>", name),
                    Err(err) => error!("{}: <{}>", err, name),
                }
            }
        }

        match archive.add_nix_profile(Path::new(NIX_PROFILE_DIR)) {
            Ok(()) => info!("Added Nix profile diagnostics"),
            Err(err) => error!("{}: Nix profile diagnostics", err),
        }

        // include files from the filesystem
        for path in self.fs_paths.iter().map(Path::new) {
            if !path.is_absolute() {
                error!("Skipping non-absolute fs path: {}", path.display());
                continue;
            }

            let entries = WalkDir::new(path)
                .into_iter()
                .filter_map(|res| res.map_err(|err| warn!("{}", err)).ok())
                .filter(|entry| entry.path().is_file());

            for entry in entries {
                match archive.add_fs_file(entry.path()) {
                    Ok(()) => info!("Added file {}", entry.path().display()),
                    Err(err) => error!("{}: {}", err, entry.path().display()),
                }
            }
        }

        // Capture the system log last, so syslog emitted by every diagnostic above
        // (e.g. a DNS error during the reachability probe) is present in the dump.
        match archive.add_cmd_output(LOGREAD_COMMAND) {
            Ok(()) => info!("Added output of '{}'", LOGREAD_COMMAND.join(" ")),
            Err(err) => error!("{}: '{}'", err, LOGREAD_COMMAND.join(" ")),
        }

        archive.finish()?;

        Ok(())
    }
}

/// Streaming writer for the support archive: entries are written to the
/// wrapped output as they are produced instead of buffering the archive.
pub struct SupportArchive<'a> {
    zip: ZipWriter<StreamWriter<Box<dyn FinishWrite + 'a>>>,
    options: SimpleFileOptions,
    filters: &'a [&'a dyn SupportFilter],
}

impl std::fmt::Debug for SupportArchive<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupportArchive").finish_non_exhaustive()
    }
}

impl<'a> SupportArchive<'a> {
    pub fn new(
        writer: impl Write + 'a,
        format: &dyn ArchiveFormat,
        compress: bool,
        filters: &'a [&'a dyn SupportFilter],
    ) -> Self {
        let options = format.file_options(SimpleFileOptions::default().compression_method(
            if compress {
                CompressionMethod::Deflated
            } else {
                CompressionMethod::Stored
            },
        ));

        Self {
            zip: ZipWriter::new_stream(format.wrap(Box::new(writer))),
            options,
            filters,
        }
    }

    pub fn finish(self) -> Result<()> {
        self.zip.finish()?.into_inner().finish()?;
        Ok(())
    }

    pub fn add_fs_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        if filters::is_excluded(self.filters, path) {
            info!("Skipped excluded file {}", path.display());
            return Ok(());
        }

        let name = Path::new("filesystem")
            .join(path.strip_prefix("/")?)
            .display()
            .to_string();

        let mut file = File::open(path)?;

        if filters::is_censored(self.filters, path) {
            let mut buf = vec![];
            file.read_to_end(&mut buf)?;
            let buf = filters::censor(self.filters, path, buf);
            self.write_file(&name, &buf)?;
        } else {
            self.zip.start_file(&name, self.options)?;
            io::copy(&mut file, &mut self.zip)?;
        }

        Ok(())
    }

    /// Add the Nix profile state summary and every generation manifest.
    ///
    /// The profile directory is read without holding the profile lock: a
    /// support archive is typically requested when the device is already
    /// misbehaving, and a stuck upgrade holding that lock is one of the
    /// failure modes we must still be able to diagnose. Blocking on the
    /// lock (or contending with a live upgrade) is the greater risk, so we
    /// accept a possibly-inconsistent snapshot instead — the same reasoning
    /// applies to the Nix database read. Collection stays best-effort: a
    /// concurrently removed generation is skipped, not fatal.
    pub fn add_nix_profile(&mut self, profile_dir: &Path) -> Result<()> {
        let state = read_nix_profile_state(profile_dir);
        self.add_builtin("nix_profile_state", &state.summary)?;

        for manifest_path in state.manifests {
            match self.add_fs_file(&manifest_path) {
                Ok(()) => info!("Added Nix profile manifest {}", manifest_path.display()),
                Err(err) => error!("{}: {}", err, manifest_path.display()),
            }
        }

        Ok(())
    }

    pub fn add_cmd_output(&mut self, cmdline: &[&str]) -> Result<()> {
        let (&program, args) = cmdline.split_first().expect("BUG: empty command");

        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let name = cmdline.join("_").replace(['/', '.'], "_");

        // NOTE: drain stderr concurrently to avoid a pipe-buffer deadlock: if
        // the child fills the stderr pipe (~64KB) before finishing stdout, it
        // blocks on the stderr write while we block reading stdout.
        let mut stderr_pipe = child.stderr.take();
        let stderr_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(stderr) = stderr_pipe.as_mut() {
                let _ = stderr.read_to_end(&mut buf);
            }
            buf
        });

        let stdout_name = format!("command/{name}");
        self.zip.start_file(&stdout_name, self.options)?;
        if let Some(mut stdout) = child.stdout.take() {
            io::copy(&mut stdout, &mut self.zip)?;
        }

        child.wait()?;
        let stderr = stderr_handle
            .join()
            .expect("BUG: stderr drain thread panicked");
        if !stderr.is_empty() {
            let stderr_name = format!("command/{name}.stderr");
            self.write_file(&stderr_name, &stderr)?;
        }

        Ok(())
    }

    pub fn add_builtin(&mut self, name: impl AsRef<str>, content: impl AsRef<str>) -> Result<()> {
        let file_name = format!("builtin/{}", name.as_ref());
        self.write_file(&file_name, content.as_ref().as_bytes())?;
        Ok(())
    }

    fn write_file(&mut self, name: &str, buf: &[u8]) -> Result<()> {
        self.zip.start_file(name, self.options)?;
        self.zip.write_all(buf)?;
        Ok(())
    }
}

/// Parse the generation number out of a `<N>-link` profile entry name.
///
/// Returns the numeric value so callers can order generations correctly;
/// `10-link` must sort after `2-link`, not before it.
fn parse_generation_name(name: &str) -> Option<usize> {
    let generation = name.strip_suffix("-link")?;
    if generation.is_empty() || !generation.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    generation.parse().ok()
}

fn symlink_target(path: &Path) -> String {
    match std::fs::read_link(path) {
        Ok(target) => target.display().to_string(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "missing".to_owned(),
        Err(err) => format!("error: {err}"),
    }
}

/// One read of the profile directory, feeding both the state summary and
/// the list of generation manifests to archive.
struct NixProfileState {
    summary: String,
    manifests: Vec<PathBuf>,
}

/// Read the profile directory once, producing the human-readable state
/// summary and the ordered set of present generation manifest paths.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "`.tmp` profile entries follow an exact lowercase naming convention"
)]
fn read_nix_profile_state(profile_dir: &Path) -> NixProfileState {
    let mut lines = vec![format!("profile_dir: {}", profile_dir.display())];

    let Ok(entries) = std::fs::read_dir(profile_dir) else {
        lines.push("status: missing".to_owned());
        return NixProfileState {
            summary: format!("{}\n", lines.join("\n")),
            manifests: Vec::new(),
        };
    };

    lines.push("status: present".to_owned());

    let mut current = None;
    let mut generations: Vec<(usize, String)> = Vec::new();
    let mut manifests: Vec<(usize, PathBuf)> = Vec::new();
    let mut next_links = Vec::new();
    let mut temporaries = Vec::new();
    let mut others = Vec::new();

    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();

        if name == "current" {
            current = Some(format!("current: {}", symlink_target(&path)));
        } else if name == "next" || name.starts_with("next.") {
            next_links.push(format!("{name}: {}", symlink_target(&path)));
        } else if let Some(number) = parse_generation_name(&name) {
            let manifest_path = path.join("manifest");
            let present = manifest_path.is_file();
            let state = if present { "present" } else { "missing" };
            generations.push((number, format!("{name}: manifest={state}")));
            if present {
                manifests.push((number, manifest_path));
            }
        } else if name.ends_with(".tmp") {
            temporaries.push(format!("temporary: {name}"));
        } else {
            others.push(format!("other: {name}"));
        }
    }

    generations.sort_by_key(|(number, _)| *number);
    manifests.sort_by_key(|(number, _)| *number);
    next_links.sort();
    temporaries.sort();
    others.sort();

    lines.push(current.unwrap_or_else(|| "current: missing".to_owned()));
    lines.extend(next_links);
    lines.extend(generations.into_iter().map(|(_, line)| line));
    lines.extend(temporaries);
    lines.extend(others);

    NixProfileState {
        summary: format!("{}\n", lines.join("\n")),
        manifests: manifests.into_iter().map(|(_, path)| path).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Cursor, Read as _};
    use std::path::Path;

    use zip::ZipArchive;

    use super::*;

    fn archive_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("BUG: test archive opens");
        let mut entries = BTreeMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).expect("BUG: archive entry exists");
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .expect("BUG: archive entry reads");
            entries.insert(file.name().to_owned(), buf);
        }
        entries
    }

    fn nix_profile_archive(profile_dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut buf = Vec::new();
        let mut archive = SupportArchive::new(&mut buf, &PlainZip, false, &[]);
        archive
            .add_nix_profile(profile_dir)
            .expect("BUG: add nix profile");
        archive.finish().expect("BUG: finish archive");
        archive_entries(&buf)
    }

    #[test]
    fn config_builder_records_each_set_field() {
        struct DummyFilter;
        impl SupportFilter for DummyFilter {}

        const COMMANDS: &[&[&str]] = &[&["echo", "hi"]];
        const FS_PATHS: &[&str] = &["/etc/hosts"];
        const HOSTS: &[&str] = &["127.0.0.1"];
        let filters: &[&dyn SupportFilter] = &[&DummyFilter];

        let config = SupportConfig::new()
            .commands(COMMANDS)
            .fs_paths(FS_PATHS)
            .ping_hosts(HOSTS)
            .filters(filters);

        assert_eq!(config.commands, COMMANDS);
        assert_eq!(config.fs_paths, FS_PATHS);
        assert_eq!(config.ping_hosts, HOSTS);
        assert_eq!(config.filters.len(), 1);
        // An untouched field stays empty.
        assert!(SupportConfig::new().commands(COMMANDS).fs_paths.is_empty());
    }

    #[test]
    fn add_cmd_output_captures_stdout_and_stderr() {
        let mut buf = Vec::new();
        let mut archive = SupportArchive::new(&mut buf, &PlainZip, false, &[]);
        archive
            .add_cmd_output(&["sh", "-c", "echo out; echo err >&2"])
            .expect("BUG: add_cmd_output");
        archive.finish().expect("BUG: finish archive");

        let entries = archive_entries(&buf);
        let stdout = entries
            .iter()
            .find(|(name, _)| name.starts_with("command/") && !name.ends_with(".stderr"))
            .map(|(_, content)| String::from_utf8_lossy(content).into_owned())
            .expect("BUG: stdout entry present");
        let stderr = entries
            .iter()
            .find(|(name, _)| name.ends_with(".stderr"))
            .map(|(_, content)| String::from_utf8_lossy(content).into_owned())
            .expect("BUG: stderr entry present");

        assert!(stdout.contains("out"), "stdout entry: {stdout:?}");
        assert!(stderr.contains("err"), "stderr entry: {stderr:?}");
    }

    #[test]
    fn archive_includes_nix_profile_manifests_without_recursing_into_generations() {
        let td = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = td.path().join("profiles/bmc");
        std::fs::create_dir_all(profile_dir.join("1-link/bin")).expect("BUG: create profile");
        std::fs::write(profile_dir.join("1-link/manifest"), "one").expect("BUG: write manifest");
        std::fs::write(profile_dir.join("1-link/bin/not-archived"), "payload")
            .expect("BUG: write payload");
        std::fs::create_dir_all(profile_dir.join("2-link")).expect("BUG: create second generation");
        std::fs::write(profile_dir.join("2-link/manifest"), "two")
            .expect("BUG: write second manifest");
        std::fs::create_dir_all(profile_dir.join("not-a-generation"))
            .expect("BUG: create non-generation");
        std::fs::write(profile_dir.join("not-a-generation/manifest"), "ignore")
            .expect("BUG: write ignored manifest");

        let entries = nix_profile_archive(&profile_dir);

        assert_eq!(
            entries
                .iter()
                .find(|(name, _)| name.ends_with("/1-link/manifest"))
                .map(|(_, content)| content.as_slice()),
            Some(&b"one"[..])
        );
        assert_eq!(
            entries
                .iter()
                .find(|(name, _)| name.ends_with("/2-link/manifest"))
                .map(|(_, content)| content.as_slice()),
            Some(&b"two"[..])
        );
        assert!(
            !entries
                .keys()
                .any(|name| name.ends_with("/bin/not-archived"))
        );
        assert!(
            !entries
                .keys()
                .any(|name| name.ends_with("/not-a-generation/manifest"))
        );
    }

    #[test]
    fn profile_state_summary_reports_symlinks_missing_manifests_and_temp_entries() {
        let td = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = td.path().join("profiles/bmc");
        std::fs::create_dir_all(profile_dir.join("1-link")).expect("BUG: create first generation");
        std::fs::write(profile_dir.join("1-link/manifest"), "one").expect("BUG: write manifest");
        std::fs::create_dir_all(profile_dir.join("2-link")).expect("BUG: create second generation");
        std::fs::create_dir_all(profile_dir.join("3-link.tmp"))
            .expect("BUG: create temp generation");
        std::fs::write(profile_dir.join("unrelated"), "x").expect("BUG: create unrelated entry");
        std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
            .expect("BUG: create current symlink");
        std::os::unix::fs::symlink("2-link", profile_dir.join("next"))
            .expect("BUG: create next symlink");
        std::os::unix::fs::symlink("3-link", profile_dir.join("next.boot"))
            .expect("BUG: create named next symlink");

        let summary = read_nix_profile_state(&profile_dir).summary;

        assert!(summary.contains("status: present"));
        assert!(summary.contains("current: 1-link"));
        assert!(summary.contains("next: 2-link"));
        assert!(summary.contains("next.boot: 3-link"));
        assert!(summary.contains("1-link: manifest=present"));
        assert!(summary.contains("2-link: manifest=missing"));
        assert!(summary.contains("temporary: 3-link.tmp"));
        assert!(summary.contains("other: unrelated"));
    }

    #[test]
    fn profile_state_summary_orders_generations_numerically() {
        let td = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = td.path().join("profiles/bmc");
        // Create out of order and past ten, where a lexicographic sort would
        // place `10-link` between `1-link` and `2-link`.
        for generation in [2_usize, 10, 1] {
            let dir = profile_dir.join(format!("{generation}-link"));
            std::fs::create_dir_all(&dir).expect("BUG: create generation");
            std::fs::write(dir.join("manifest"), generation.to_string())
                .expect("BUG: write manifest");
        }

        let summary = read_nix_profile_state(&profile_dir).summary;

        let position = |needle: &str| {
            summary
                .find(needle)
                .expect("BUG: generation line present in summary")
        };
        assert!(
            position("1-link: manifest=present") < position("2-link: manifest=present")
                && position("2-link: manifest=present") < position("10-link: manifest=present"),
            "generations must be ordered numerically, not lexicographically:\n{summary}"
        );
    }

    #[test]
    fn archive_records_missing_profile_dir_in_summary_without_failing() {
        let td = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = td.path().join("missing/bmc");

        let entries = nix_profile_archive(&profile_dir);

        let summary = String::from_utf8(
            entries
                .get("builtin/nix_profile_state")
                .expect("BUG: profile summary exists")
                .clone(),
        )
        .expect("BUG: summary is utf8");

        assert!(summary.contains("status: missing"));
    }
}
