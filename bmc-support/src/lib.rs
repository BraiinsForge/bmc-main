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

pub use filters::{SupportFilter, censor};
pub use format::{ArchiveFormat, FinishWrite, PasswordProtectedZip, PlainZip};

use anyhow::Result;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
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

/// A binary-specific collector run during archive collection.
///
/// Implementations write their diagnostics into the archive; they run after
/// the built-in collection steps, in registration order.
pub trait SupportExtension: Sync {
    /// Short name, used for logging.
    fn name(&self) -> &'static str;

    /// Write this collector's diagnostics into `archive`.
    fn collect(&self, archive: &mut SupportArchive<'_>) -> Result<()>;
}

pub struct SupportConfig<'a> {
    commands: &'a [&'a [&'a str]],
    fs_paths: &'a [&'a str],
    ping_hosts: &'a [&'a str],
    filters: &'a [&'a dyn SupportFilter],
    extensions: &'a [&'a dyn SupportExtension],
}

impl std::fmt::Debug for SupportConfig<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupportConfig")
            .field("commands", &self.commands)
            .field("fs_paths", &self.fs_paths)
            .field("ping_hosts", &self.ping_hosts)
            .field("filters", &self.filters.len())
            .field("extensions", &self.extensions.len())
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
            extensions: &[],
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

    /// Binary-specific collectors, run after the fs walk in registration order
    #[must_use]
    pub const fn extensions(mut self, extensions: &'a [&'a dyn SupportExtension]) -> Self {
        self.extensions = extensions;
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

        // Commands run before the fs walk so syslog they emit (e.g. dnsmasq)
        // lands in the log file the walk collects.
        for &cmdline in self.commands {
            match archive.add_cmd_output(cmdline) {
                Ok(()) => info!("Added output of '{}'", cmdline.join(" ")),
                Err(err) => error!("{}: '{}'", err, cmdline.join(" ")),
            }
        }

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

        // Extensions run last so a log-capturing extension sees syslog emitted
        // by every earlier step (e.g. a DNS error during the reachability probe).
        for extension in self.extensions {
            match extension.collect(&mut archive) {
                Ok(()) => info!("Ran extension <{}>", extension.name()),
                Err(err) => error!("{}: extension <{}>", err, extension.name()),
            }
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

    /// Add a raw archive entry at `name` with `content`, for extensions that
    /// place files under their own path prefix.
    pub fn add_entry(&mut self, name: &str, content: &[u8]) -> Result<()> {
        self.write_file(name, content)
    }

    fn write_file(&mut self, name: &str, buf: &[u8]) -> Result<()> {
        self.zip.start_file(name, self.options)?;
        self.zip.write_all(buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Cursor, Read as _};

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
    fn extension_writes_through_the_archive() {
        struct DummyExtension;
        impl SupportExtension for DummyExtension {
            fn name(&self) -> &'static str {
                "dummy"
            }
            fn collect(&self, archive: &mut SupportArchive<'_>) -> Result<()> {
                archive.add_builtin("x", "hello")?;
                archive.add_entry("custom/y", b"world")?;
                Ok(())
            }
        }

        let mut buf = Vec::new();
        let mut archive = SupportArchive::new(&mut buf, &PlainZip, false, &[]);
        DummyExtension
            .collect(&mut archive)
            .expect("BUG: extension collect");
        archive.finish().expect("BUG: finish archive");

        let entries = archive_entries(&buf);
        assert_eq!(
            entries.get("builtin/x").map(Vec::as_slice),
            Some(&b"hello"[..])
        );
        assert_eq!(
            entries.get("custom/y").map(Vec::as_slice),
            Some(&b"world"[..])
        );
    }

    #[test]
    fn archive_entry_names_keep_prefix_and_insertion_order() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"aaa").expect("BUG: write temp file");

        let mut buf = Vec::new();
        let mut archive = SupportArchive::new(&mut buf, &PlainZip, false, &[]);
        archive
            .add_cmd_output(&["echo", "hi"])
            .expect("BUG: add_cmd_output");
        archive
            .add_builtin("timestamp", "123")
            .expect("BUG: add_builtin");
        archive.add_fs_file(&file).expect("BUG: add_fs_file");
        archive.finish().expect("BUG: finish archive");

        let mut zip = ZipArchive::new(Cursor::new(&buf)).expect("BUG: open archive");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).expect("BUG: entry").name().to_owned())
            .collect();

        assert_eq!(names.len(), 3, "entries: {names:?}");
        assert_eq!(names[0], "command/echo_hi");
        assert_eq!(names[1], "builtin/timestamp");
        assert!(
            names[2].starts_with("filesystem/") && names[2].ends_with("a.txt"),
            "fs entry: {}",
            names[2]
        );
    }
}
