// Copyright (C) 2026  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

//! Small process-spawning helpers shared by the platform network managers.
//!
//! Every spawn goes through [`run_command`], so the timeout, the `stdin`
//! handling and the kill-on-timeout policy hold for all of them: a helper that
//! bypassed it would be the one place a wedged child could still pin a request.
//! Service restarts are the one exemption, see [`call_command_unbounded`].

use std::process::{Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Shell library providing the `bos-defaults` flag helpers and predicates.
pub(crate) const BOS_DEFAULTS_LIB: &str = "/lib/functions/bos-defaults.sh";
/// Shell library providing the factory-default / captive-portal helpers.
pub(crate) const BOS_FACTORY_DEFAULT_LIB: &str = "/lib/functions/bos-factory-default.sh";

/// Upper bound on a single system command, so a wedged child (e.g. one that
/// inherited a `udhcpc` pipe and never sees EOF) can't hang the request forever.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Spawn `command` with `args`, feed it `stdin` if given, and collect its
/// [`Output`], within `timeout` when one is given.
///
/// `stdin` is `/dev/null` unless a payload is supplied, so a child that reads
/// its input never waits on ours. On timeout the direct child is killed (the
/// `Child` is dropped with `kill_on_drop`) rather than left running with the
/// pipe. Its own children are not: that is why anything that spawns a
/// service's process tree runs unbounded.
///
/// A supplied `stdin` payload is written in full before the output is drained,
/// so it must stay small: one large enough for the child to fill its
/// stdout/stderr pipe (~64 KiB) while still reading stdin would deadlock. The
/// `uci batch` callers only ever feed a handful of short lines.
async fn run_command(
    command: &str,
    args: &[&str],
    stdin: Option<&str>,
    timeout: Option<Duration>,
) -> Result<Output> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{command}`"))?;

    if let Some(payload) = stdin {
        let mut child_stdin = child.stdin.take().context("BUG: stdin was piped")?;
        child_stdin.write_all(payload.as_bytes()).await?;
        child_stdin.flush().await?;
        drop(child_stdin);
    }

    let output = child.wait_with_output();
    let output = match timeout {
        Some(limit) => tokio::time::timeout(limit, output).await.map_err(|_| {
            tracing::error!(
                "`{command} {}` did not finish within {limit:?}, killing it",
                args.join(" ")
            );
            anyhow!("`{command}` timed out after {limit:?}")
        })?,
        None => output.await,
    };
    output.with_context(|| format!("waiting for `{command}`"))
}

/// Run `command` with `args`, discarding stdout; error if it exits non-zero.
pub(crate) async fn call_command(command: &str, args: &[&str]) -> Result<()> {
    call_command_to_string(command, args).await.map(|_| ())
}

/// [`call_command`] without the timeout, for service restarts.
///
/// An init script's `restart` forks a process tree (`udhcpc`, `dnsmasq`, ...)
/// that outlives the script and does not die with it, and killing the script
/// between its `stop` and `start` leaves the box with networking down. Both
/// are worse than waiting, so the bound is dropped rather than raised.
pub(crate) async fn call_command_unbounded(command: &str, args: &[&str]) -> Result<()> {
    succeed_or_bail(command, &run_command(command, args, None, None).await?).map(|_| ())
}

/// Run `command` with `args` and return its stdout; error if it exits non-zero.
pub(crate) async fn call_command_to_string(command: &str, args: &[&str]) -> Result<String> {
    succeed_or_bail(
        command,
        &run_command(command, args, None, Some(COMMAND_TIMEOUT)).await?,
    )
}

fn succeed_or_bail(command: &str, output: &Output) -> Result<String> {
    if !output.status.success() {
        bail!(
            "command `{command}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run `command` with `args` and report whether it exited successfully.
///
/// Unlike [`call_command`], a non-zero exit is *not* an error — it is reported
/// as `Ok(false)`. `Err` is reserved for the command failing to launch at all
/// (e.g. the binary is missing) or to finish in time, letting callers
/// distinguish "ran and answered no" from "could not be evaluated".
pub(crate) async fn command_succeeds(command: &str, args: &[&str]) -> Result<bool> {
    Ok(run_command(command, args, None, Some(COMMAND_TIMEOUT))
        .await?
        .status
        .success())
}

/// Run `command` with `args`, feeding `stdin` to its standard input, and return
/// the captured [`Output`] (caller inspects status/stderr).
pub(crate) async fn call_command_stdin(
    command: &str,
    args: &[&str],
    stdin: &str,
) -> Result<Output> {
    run_command(command, args, Some(stdin), Some(COMMAND_TIMEOUT)).await
}

/// Build the `sh -c` script that sources `lib` and then runs `snippet`.
fn sourced_script(lib: &str, snippet: &str) -> String {
    format!(". {lib} && {snippet}")
}

/// Source `lib` and run `snippet`, returning its stdout; error on non-zero exit.
pub(crate) async fn run_sourced_to_string(lib: &str, snippet: &str) -> Result<String> {
    call_command_to_string("sh", &["-c", &sourced_script(lib, snippet)]).await
}

/// Source `lib` and run `snippet`, discarding stdout; error on non-zero exit.
pub(crate) async fn run_sourced(lib: &str, snippet: &str) -> Result<()> {
    run_sourced_to_string(lib, snippet).await.map(|_| ())
}

/// Source `lib` and run `snippet`, reporting whether it exited successfully.
///
/// A non-zero exit is `Ok(false)`; `Err` is reserved for the command failing to
/// launch (see [`command_succeeds`]).
pub(crate) async fn run_sourced_succeeds(lib: &str, snippet: &str) -> Result<bool> {
    command_succeeds("sh", &["-c", &sourced_script(lib, snippet)]).await
}
