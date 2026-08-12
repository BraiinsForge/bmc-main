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

pub mod constants;
pub mod encrypt;
mod filters;
pub use bmc_net_diag as network;

use crate::constants::{
    BMC_CONFIG_DIR, BMC_CONFIG_LEGACY, BOARD, BOS_MAJOR, BOS_MODE, BOS_PLATFORM, BOS_VERSION,
    ETC_DNSMASQ_CONF, ETC_HOSTS, ETC_RESOLV_CONF, FACTORY_DEFAULT, PROC_CPUINFO, PROC_MTD,
    SETUP_PENDING, SRC_ETC_CONF, SRC_LOGS,
};
use crate::network::PcapResult;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use walkdir::WalkDir;
use zip::result::ZipResult;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const PCAP_DURATION: Duration = Duration::from_secs(5);
const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";

/// These commands will be executed and their stdout will be included in the support archive.
const COMMANDS: &[&[&str]] = &[
    &["dmesg"],
    &["fw_printenv"],
    &["env"],
    &["ifconfig", "-a"],
    &["ip", "addr"],
    &["ip", "route"],
    &["ps", "aux"],
    &["df"],
    &["ls", "-l", "/tmp"],
    &["killall", "-SIGUSR1", "dnsmasq"],
];

/// Captured after every other diagnostic (see [`collect`]) so syslog that
/// they trigger — e.g. dnsmasq reacting to the reachability probe — is
/// included in the snapshot.
const LOGREAD_COMMAND: &[&str] = &["logread"];

/// All contents of these paths will be included in the support archive.
const FS_PATHS: &[&str] = &[
    // files
    BOS_VERSION,
    BOS_MAJOR,
    BOS_MODE,
    BOS_PLATFORM,
    ETC_HOSTS,
    ETC_RESOLV_CONF,
    ETC_DNSMASQ_CONF,
    BOARD,
    // Pre-migration config, kept on disk for downgrade safety. The
    // current config and its timestamped backups come in via the
    // BMC_CONFIG_DIR directory below.
    BMC_CONFIG_LEGACY,
    FACTORY_DEFAULT,
    SETUP_PENDING,
    // directories
    BMC_CONFIG_DIR,
    SRC_LOGS,
    SRC_ETC_CONF,
    "/etc/nix-upgrade",
    "/etc/nix/nix.conf",
    // additional procfs items
    PROC_MTD,
    PROC_CPUINFO,
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

pub const PING_HOSTS: &[&str] = &[
    "127.0.0.1",
    "8.8.8.8",
    "google.com",
    "downloads.braiins.com",
    "downloads.braiinsforge.com",
    "public-api.braiins.com",
];

#[derive(Debug)]
pub enum SupportArchiveFormat {
    Zip,
    ZipEncrypted,
}

pub fn collect(
    writer: &mut impl Write,
    format: SupportArchiveFormat,
    compress: bool,
) -> Result<()> {
    let mut archive = SupportZipWriter::new(writer, format, compress);

    // include outputs of commands
    // These run before the log capture below since some of them emit syslog (e.g. dnsmasq).
    for &cmdline in COMMANDS {
        match archive.add_cmd_output(cmdline) {
            Ok(()) => info!("Added output of '{}'", cmdline.join(" ")),
            Err(err) => error!("{}: '{}'", err, cmdline.join(" ")),
        }
    }

    // Start a pcap capture on every interface concurrently; enumeration and
    // threading live in bmc-net-diag so this crate stays free of pnet.
    let pcap_capture = network::pcap_all(PCAP_DURATION);

    // include output of builtin routines
    // Again these commands may produce some logs so log collection must be done after this.
    #[expect(clippy::type_complexity)]
    let builtin_items: &[(&str, fn() -> Option<String>)] = &[
        ("ifconfig", || Some(network::ifconfig())),
        ("public_ip", || network::public_ip().ok()),
        ("ping_report", || network::ping_report(PING_HOSTS).ok()),
        ("timestamp", || {
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
    for path in FS_PATHS.iter().map(Path::new) {
        assert!(path.is_absolute());

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

    // collect pcap results and include them as well
    for pcap_result in pcap_capture.collect() {
        match archive.add_pcap(pcap_result) {
            Ok(interface_name) => info!("Added pcap of interface '{}'", interface_name),
            Err(err) => error!("Error adding pcap: {:#}", err),
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

struct SupportZipWriter<'w, W: Write> {
    writer: &'w mut W,
    format: SupportArchiveFormat,
    archive: ZipWriter<Cursor<Vec<u8>>>,
    options: SimpleFileOptions,
}

impl<'w, W: Write> SupportZipWriter<'w, W> {
    pub fn new(writer: &'w mut W, format: SupportArchiveFormat, compress: bool) -> Self {
        let options = SimpleFileOptions::default().compression_method(if compress {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        });

        Self {
            writer,
            format,
            archive: ZipWriter::new(Cursor::new(vec![])),
            options,
        }
    }

    pub fn finish(self) -> ZipResult<()> {
        let mut buffer = self.archive.finish()?.into_inner();

        buffer = match self.format {
            SupportArchiveFormat::ZipEncrypted => encrypt::encrypt(&buffer),
            SupportArchiveFormat::Zip => buffer,
        };

        self.writer.write_all(&buffer)?;

        Ok(())
    }

    pub fn add_fs_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        if filters::is_excluded(path) {
            info!("Skipped excluded file {}", path.display());
            return Ok(());
        }

        let mut file = File::open(path)?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)?;

        let buf = filters::apply(path, buf);

        let name = Path::new("filesystem")
            .join(path.strip_prefix("/")?)
            .display()
            .to_string();

        self.write_file(&name, &buf)?;

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

        let output = Command::new(program).args(args).output()?;

        let name = cmdline.join("_").replace(['/', '.'], "_");

        let stdout = format!("command/{name}");
        self.write_file(&stdout, &output.stdout)?;

        if !output.stderr.is_empty() {
            let stderr = format!("command/{name}.stderr");
            self.write_file(&stderr, &output.stderr)?;
        }

        Ok(())
    }

    pub fn add_builtin(&mut self, name: impl AsRef<str>, content: impl AsRef<str>) -> Result<()> {
        let file_name = format!("builtin/{}", name.as_ref());
        self.write_file(&file_name, content.as_ref().as_bytes())?;
        Ok(())
    }

    pub fn add_pcap(&mut self, pcap_result: PcapResult) -> Result<String> {
        let (interface_name, pcap) = pcap_result;
        let pcap = pcap.with_context(|| format!("capture on '{interface_name}' failed"))?;
        let file_name = format!("pcap/{interface_name}.pcap");
        self.write_file(&file_name, &pcap)?;
        Ok(interface_name)
    }

    fn write_file(&mut self, name: &str, buf: &[u8]) -> Result<()> {
        self.archive.start_file(name, self.options)?;
        self.archive.write_all(buf)?;
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
        let mut archive = SupportZipWriter::new(&mut buf, SupportArchiveFormat::Zip, false);
        archive
            .add_nix_profile(profile_dir)
            .expect("BUG: add nix profile");
        archive.finish().expect("BUG: finish archive");
        archive_entries(&buf)
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
