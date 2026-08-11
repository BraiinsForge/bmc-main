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

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositorIdentity {
    pub boot_id: String,
    pub pid: libc::pid_t,
    pub starttime: u64,
}

impl fmt::Display for CompositorIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} {}",
            self.boot_id, self.pid, self.starttime
        )
    }
}

impl FromStr for CompositorIdentity {
    type Err = anyhow::Error;

    fn from_str(record: &str) -> Result<Self> {
        let mut fields = record.split_whitespace();
        let boot_id = fields.next().context("ownership record has no boot id")?;
        let pid = fields
            .next()
            .context("ownership record has no pid")?
            .parse::<libc::pid_t>()
            .context("ownership record pid is invalid")?;
        let starttime = fields
            .next()
            .context("ownership record has no starttime")?
            .parse::<u64>()
            .context("ownership record starttime is invalid")?;
        if fields.next().is_some() {
            bail!("ownership record has extra fields");
        }
        if pid <= 0 {
            bail!("ownership record pid must be positive");
        }
        Ok(Self {
            boot_id: boot_id.to_owned(),
            pid,
            starttime,
        })
    }
}

#[derive(Debug)]
pub enum RecordStatus {
    Match,
    Missing,
    Malformed { error: anyhow::Error },
    Unreadable { error: io::Error },
    Mismatch { recorded: CompositorIdentity },
}

#[must_use]
pub fn read_record_status(path: &Path, current: &CompositorIdentity) -> RecordStatus {
    match std::fs::read_to_string(path) {
        Ok(record) => match record.parse::<CompositorIdentity>() {
            Ok(recorded) if recorded == *current => RecordStatus::Match,
            Ok(recorded) => RecordStatus::Mismatch { recorded },
            Err(error) => RecordStatus::Malformed { error },
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => RecordStatus::Missing,
        Err(error) => RecordStatus::Unreadable { error },
    }
}

pub fn current_compositor_identity() -> Result<CompositorIdentity> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .context("read kernel boot id")?;
    let boot_id = boot_id.trim();
    if boot_id.is_empty() {
        bail!("kernel boot id is empty");
    }
    let pid = libc::pid_t::try_from(std::os::unix::process::parent_id())
        .context("compositor parent pid does not fit pid_t")?;
    if pid == 0 {
        bail!("compositor parent is outside our pid namespace");
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("read compositor process stat for pid {pid}"))?;
    let starttime = parse_proc_stat_starttime(&stat)
        .with_context(|| format!("parse compositor process stat for pid {pid}"))?;
    Ok(CompositorIdentity {
        boot_id: boot_id.to_owned(),
        pid,
        starttime,
    })
}

pub fn parse_proc_stat_starttime(stat: &str) -> Result<u64> {
    proc_stat_fields(stat)?
        .split_whitespace()
        .nth(19)
        .context("process stat has no starttime field")?
        .parse::<u64>()
        .context("process stat starttime is invalid")
}

pub fn proc_stat_is_zombie(stat: &str) -> Result<bool> {
    Ok(proc_stat_fields(stat)?
        .split_whitespace()
        .next()
        .context("process stat has no state field")?
        == "Z")
}

fn proc_stat_fields(stat: &str) -> Result<&str> {
    let close = stat
        .rfind(')')
        .context("process stat has no closing process-name parenthesis")?;
    stat.get(close + 1..)
        .context("process stat process-name boundary is invalid")
}

pub fn commit_record(path: &Path, identity: &CompositorIdentity) -> Result<()> {
    let temp = temporary_record_path(path);
    let result = write_and_rename_record(&temp, path, identity);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn write_and_rename_record(temp: &Path, path: &Path, identity: &CompositorIdentity) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(temp)
        .with_context(|| format!("open temporary ownership record {}", temp.display()))?;
    writeln!(file, "{identity}")
        .with_context(|| format!("write temporary ownership record {}", temp.display()))?;
    file.flush()
        .with_context(|| format!("flush temporary ownership record {}", temp.display()))?;
    drop(file);
    std::fs::rename(temp, path).with_context(|| {
        format!(
            "rename temporary ownership record {} to {}",
            temp.display(),
            path.display()
        )
    })
}

fn temporary_record_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    std::path::PathBuf::from(name)
}

pub fn remove_record(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
