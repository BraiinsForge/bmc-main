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

use std::process::{Output, Stdio};

use anyhow::{Context, Result, bail};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Shell library providing the `bos-defaults` flag helpers and predicates.
pub(crate) const BOS_DEFAULTS_LIB: &str = "/lib/functions/bos-defaults.sh";
/// Shell library providing the factory-default / captive-portal helpers.
pub(crate) const BOS_FACTORY_DEFAULT_LIB: &str = "/lib/functions/bos-factory-default.sh";

/// Run `command` with `args`, discarding stdout; error if it exits non-zero.
pub(crate) async fn call_command(command: &str, args: &[&str]) -> Result<()> {
    call_command_to_string(command, args).await.map(|_| ())
}

/// Run `command` with `args` and return its stdout; error if it exits non-zero.
pub(crate) async fn call_command_to_string(command: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .await
        .with_context(|| format!("spawning `{command}`"))?;
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
/// (e.g. the binary is missing), letting callers distinguish "ran and answered
/// no" from "could not be evaluated".
pub(crate) async fn command_succeeds(command: &str, args: &[&str]) -> Result<bool> {
    let output = Command::new(command)
        .args(args)
        .output()
        .await
        .with_context(|| format!("spawning `{command}`"))?;
    Ok(output.status.success())
}

/// Run `command` with `args`, feeding `stdin` to its standard input, and return
/// the captured [`Output`] (caller inspects status/stderr).
///
/// The whole `stdin` payload is written before the output is drained, so it must
/// stay small: a payload large enough for the child to fill its stdout/stderr
/// pipe (~64 KiB) while still reading stdin would deadlock. The `uci batch`
/// callers only ever feed a handful of short lines.
pub(crate) async fn call_command_stdin(
    command: &str,
    args: &[&str],
    stdin: &str,
) -> Result<Output> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning `{command}`"))?;

    let mut child_stdin = child.stdin.take().context("BUG: stdin was piped")?;
    child_stdin.write_all(stdin.as_bytes()).await?;
    child_stdin.flush().await?;
    drop(child_stdin);

    Ok(child.wait_with_output().await?)
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
