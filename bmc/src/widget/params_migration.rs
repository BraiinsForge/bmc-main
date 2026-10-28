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

//! Run a package's params migrator for one widget instance
//! whose stored params predate the installed package version,
//! and commit the result. Protocol 0: the program reads one JSON object
//! on stdin and answers one JSON object on stdout,
//! an overlay of params keys where `{}` means the stored shape is current.
//! Without a migration, defaults are never written:
//! a widget launches on its params read through the installed manifest,
//! see [`launch_params`].

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bmc_field_schema::{MissingValues, Violation};
use bmc_widget_manifest::{Manifest, ParamKey, ParamValue};
use bmc_widget_protocol::{DisplayInfo, ViewportShape};
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::WidgetInfo;
use crate::config::ConfigHandle;
use crate::scene::{SceneId, Widget, WidgetId};

/// Wall-clock budget for one program run; the process is killed past it.
/// Raise it if a shipped migrator legitimately needs longer.
pub(crate) const MIGRATOR_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest stdout the runner reads; more is a failure.
/// Raise it if a shipped migrator legitimately emits more.
pub(crate) const MIGRATOR_STDOUT_LIMIT: usize = 256 * 1024;
/// Longest stderr the runner keeps for the log; the rest is drained.
pub(crate) const MIGRATOR_STDERR_LIMIT: usize = 64 * 1024;
/// How many migrator processes a gate runs at once:
/// enough to keep the gate short, few enough to bound their memory.
pub(crate) const MIGRATOR_CONCURRENCY: usize = 4;
/// How much of the program's stdout a failure log line quotes.
const MIGRATOR_PREVIEW_CHARS: usize = 120;
/// The one stdin/stdout contract this build implements.
pub(crate) const SUPPORTED_MIGRATOR_PROTOCOL: u32 = 0;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MigrationFailure {
    #[error("failed to spawn the migrator: {0}")]
    Spawn(std::io::Error),
    #[error("i/o with the migrator failed: {0}")]
    Io(std::io::Error),
    #[error("the migrator did not finish within {MIGRATOR_TIMEOUT:?} and was killed")]
    Timeout,
    #[error("the migrator exited with {0}")]
    Exit(std::process::ExitStatus),
    #[error("the migrator wrote more than {MIGRATOR_STDOUT_LIMIT} bytes")]
    StdoutTooLarge,
    #[error("the migrator did not write a JSON object")]
    NotAnObject,
    #[error("the migrated params failed validation: {}", violations_line(.0))]
    Rejected(Vec<Violation>),
}

fn violations_line(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|violation| format!("{}: {}", violation.path, violation.message))
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) struct MigrationJob<'a> {
    pub scene_id: SceneId,
    pub widget_id: WidgetId,
    pub stored_params: &'a BTreeMap<ParamKey, ParamValue>,
    pub params_version: Option<&'a semver::Version>,
    pub migrator: Migrator<'a>,
    pub viewport: MigratorViewport,
}

/// The geometry a migrator learns besides the params.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MigratorViewport {
    pub width: u32,
    pub height: u32,
    pub viewport_shape: ViewportShape,
    pub display: DisplayInfo,
}

/// Protocol 0's stdin: a field added here or to [`DisplayInfo`] changes the protocol.
#[derive(Serialize)]
struct MigratorInput<'a> {
    params_version: Option<&'a semver::Version>,
    widget_version: &'a semver::Version,
    params: serde_json::Map<String, serde_json::Value>,
    width: u32,
    height: u32,
    viewport_shape: &'a ViewportShape,
    display: &'a DisplayInfo,
}

/// Whether the installed package ships a migrator this build can run.
#[derive(Debug)]
pub(crate) enum MigratorStatus<'a> {
    Usable(Migrator<'a>),
    Absent,
    UnsupportedProtocol(u32),
}

/// An installed package with the migrator it ships, so a job cannot run
/// one package's program against another package's manifest.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migrator<'a> {
    installed: &'a WidgetInfo,
    path: &'a Path,
}

impl<'a> Migrator<'a> {
    pub(crate) fn installed(&self) -> &'a WidgetInfo {
        self.installed
    }
}

pub(crate) fn migrator_status(info: &WidgetInfo) -> MigratorStatus<'_> {
    let Some(migrator) = info.params_migrator.as_ref() else {
        return MigratorStatus::Absent;
    };
    if migrator.protocol != SUPPORTED_MIGRATOR_PROTOCOL {
        return MigratorStatus::UnsupportedProtocol(migrator.protocol);
    }
    MigratorStatus::Usable(Migrator {
        installed: info,
        path: &migrator.path,
    })
}

pub(crate) fn migrator_input(
    params_version: Option<&semver::Version>,
    widget_version: &semver::Version,
    params: &BTreeMap<ParamKey, ParamValue>,
    viewport: &MigratorViewport,
) -> Vec<u8> {
    serde_json::to_vec(&MigratorInput {
        params_version,
        widget_version,
        params: crate::widget::coordinator::params_to_json_map(params),
        width: viewport.width,
        height: viewport.height,
        viewport_shape: &viewport.viewport_shape,
        display: &viewport.display,
    })
    .expect("BUG: the migrator input is plain data")
}

pub(crate) async fn run_migrator(
    job: &MigrationJob<'_>,
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    let input = migrator_input(
        job.params_version,
        &job.migrator.installed.manifest.version,
        job.stored_params,
        &job.viewport,
    );

    let mut child = Command::new(job.migrator.path)
        .current_dir(&job.migrator.installed.widget_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(MigrationFailure::Spawn)?;
    let stdin = child.stdin.take().expect("BUG: stdin was piped");
    let mut stdout = child.stdout.take().expect("BUG: stdout was piped");
    let mut stderr = child.stderr.take().expect("BUG: stderr was piped");
    let widget_id = job.widget_id;

    // The buffers outlive the exchange so whatever the program said
    // before a timeout or a pipe error still reaches the log.
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    // The write runs beside the reads: fed sequentially, an input past the pipe buffer
    // wedges against a program that fills stdout or stderr before draining stdin.
    let exchange = async {
        let feed = async {
            let mut stdin = stdin;
            match stdin.write_all(&input).await {
                // A program may answer without reading its input; the answer decides.
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                fed => fed.map_err(MigrationFailure::Io),
            }
        };
        let stderr_task = read_capped(&mut stderr, MIGRATOR_STDERR_LIMIT, &mut diagnostics);
        let stdout_task = read_capped(&mut stdout, MIGRATOR_STDOUT_LIMIT + 1, &mut output);
        let (fed, stdout_read, stderr_read) = tokio::join!(feed, stdout_task, stderr_task);
        fed?;
        stdout_read?;
        stderr_read?;
        child.wait().await.map_err(MigrationFailure::Io)
    };

    let finished = match tokio::time::timeout(MIGRATOR_TIMEOUT, exchange).await {
        Ok(result) => result,
        Err(_elapsed) => {
            if let Err(error) = child.kill().await {
                warn!(%widget_id, widget = %job.migrator.installed.manifest.uid, %error, "failed to kill the timed-out migrator");
            }
            Err(MigrationFailure::Timeout)
        }
    };

    for line in String::from_utf8_lossy(&diagnostics).lines() {
        info!(%widget_id, "migrator: {line}");
    }
    let result = finished.and_then(|status| interpret_output(job, status, &output));
    if result.is_err() {
        log_stdout_preview(widget_id, &output);
    }
    result
}

fn log_stdout_preview(widget_id: WidgetId, output: &[u8]) {
    if output.is_empty() {
        return;
    }
    let preview: String = String::from_utf8_lossy(output)
        .chars()
        .take(MIGRATOR_PREVIEW_CHARS)
        .collect();
    info!(%widget_id, bytes = output.len(), "migrator stdout began with {preview:?}");
}

fn interpret_output(
    job: &MigrationJob<'_>,
    status: std::process::ExitStatus,
    output: &[u8],
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    if !status.success() {
        return Err(MigrationFailure::Exit(status));
    }
    migrated_params(&job.migrator.installed.manifest, job.stored_params, output)
}

/// The params a migrator's stdout leaves `stored` with under `manifest`.
pub(crate) fn migrated_params(
    manifest: &Manifest,
    stored: &BTreeMap<ParamKey, ParamValue>,
    output: &[u8],
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    if output.len() > MIGRATOR_STDOUT_LIMIT {
        return Err(MigrationFailure::StdoutTooLarge);
    }
    let Ok(serde_json::Value::Object(overlay)) =
        serde_json::from_slice::<serde_json::Value>(output)
    else {
        return Err(MigrationFailure::NotAnObject);
    };
    merge_overlay(manifest, stored, &overlay)
}

/// Keep at most `limit` bytes from `reader` in `buffer` and drain the rest,
/// so a chatty program neither blocks on the pipe nor grows memory.
async fn read_capped(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    limit: usize,
    buffer: &mut Vec<u8>,
) -> Result<(), MigrationFailure> {
    let limit = u64::try_from(limit).expect("BUG: a usize limit fits u64");
    reader
        .take(limit)
        .read_to_end(buffer)
        .await
        .map_err(MigrationFailure::Io)?;
    tokio::io::copy(reader, &mut tokio::io::sink())
        .await
        .map_err(MigrationFailure::Io)?;
    Ok(())
}

/// Stored params minus keys the manifest no longer declares, the overlay
/// on top, validated with defaults filling what is still missing.
pub(crate) fn merge_overlay(
    manifest: &Manifest,
    stored: &BTreeMap<ParamKey, ParamValue>,
    overlay: &serde_json::Map<String, serde_json::Value>,
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    let mut values: BTreeMap<String, Result<ParamValue, String>> = stored
        .iter()
        .filter(|(key, _)| manifest.params.contains_key(key.as_str()))
        .map(|(key, value)| (key.as_str().to_owned(), Ok(value.clone())))
        .collect();
    for (key, value) in overlay {
        values.insert(
            key.clone(),
            ParamValue::try_from(value).map_err(|error| error.to_string()),
        );
    }
    bmc_field_schema::validate_values(&manifest.params, &values, MissingValues::Default)
        .map_err(MigrationFailure::Rejected)
}

/// Whether params stamped `stamp` predate `installed`, the one case a migrator runs for.
/// A stamp newer than the package, as a downgrade leaves it, is kept as it is.
pub(crate) fn params_predate(stamp: Option<&semver::Version>, installed: &semver::Version) -> bool {
    stamp.is_none_or(|stamp| stamp < installed)
}

/// The params `widget` launches with: the stored ones read through `manifest`
/// as the settings form reads them, so undeclared keys drop and missing ones take their defaults.
/// The config keeps only what the user or a migrator wrote.
/// A widget its package's migrator still owes a migration launches on its stored params:
/// that package said defaults alone do not carry its params forward.
pub(crate) fn launch_params(
    widget: &Widget,
    manifest: &Manifest,
) -> BTreeMap<ParamKey, ParamValue> {
    let migration_owed = manifest.programs.params_migrator.is_some()
        && params_predate(widget.params_version.as_ref(), &manifest.version);
    if migration_owed {
        return widget.params.clone();
    }
    merge_overlay(manifest, &widget.params, &serde_json::Map::new()).unwrap_or_else(|failure| {
        warn!(widget_id = %widget.id, widget = %manifest.uid, %failure, "params do not fit the package; launching on the stored params");
        widget.params.clone()
    })
}

/// Store each migrated result, stamped with the installed version, under one save,
/// unless a UI save moved that widget's stamp since its job read it.
/// Returns the widgets whose result the config took.
pub(crate) async fn commit_migrated_params(
    config_handle: &Arc<RwLock<ConfigHandle>>,
    results: Vec<(MigrationJob<'_>, BTreeMap<ParamKey, ParamValue>)>,
) -> Vec<WidgetId> {
    let mut config = config_handle.write().await;
    let mut committed = Vec::new();
    for (job, params) in results {
        let MigrationJob {
            scene_id,
            widget_id,
            params_version: expected_stamp,
            migrator,
            ..
        } = job;
        let Some(stored) = config
            .edit_scene(&scene_id)
            .and_then(|scene| scene.widget(&widget_id))
        else {
            info!(%widget_id, "widget removed while its params migrated; discarding the result");
            continue;
        };
        if stored.get().params_version.as_ref() != expected_stamp {
            info!(%widget_id, "params were saved while they migrated; discarding the migrator result");
            continue;
        }
        let widget = stored.into_mut();
        widget.params = params;
        widget.params_version = Some(migrator.installed.manifest.version.clone());
        committed.push(widget_id);
    }
    if committed.is_empty() {
        return committed;
    }
    if let Err(error) = config.save().await {
        warn!(%error, "failed to save migrated params; kept in memory until the next save or boot");
    }
    committed
}

#[cfg(test)]
mod tests;
