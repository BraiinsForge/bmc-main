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

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, BufReader};

/// Longest captured line; longer lines are split at this size.
const MAX_LINE_BYTES: u64 = 8 * 1024;

/// Forward one stdio stream of a widget process line by line as
/// [`bmc_log::WIDGET_OUTPUT_TARGET`] events until EOF.
///
/// Must keep draining whatever arrives: dropping the reader early would
/// close the pipe and make the widget's own stderr writes fail with
/// EPIPE for the rest of its life. Invalid UTF-8 is replaced lossily
/// and overlong lines are split instead of treated as errors.
pub(crate) async fn forward_widget_output<R>(
    stream: R,
    widget_name: &str,
    instance_id: &str,
    pid: u32,
) where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buf)
            .await
        {
            Ok(0) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\r', '\n']);
                tracing::info!(
                    target: bmc_log::WIDGET_OUTPUT_TARGET,
                    "{widget_name}[{instance_id}/{pid}]: {line}"
                );
            }
            Err(err) => {
                // A read error means this pipe is already broken, so stop draining it.
                tracing::warn!(%widget_name, instance_id, pid, ?err, "stopped capturing widget output");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    use super::forward_widget_output;

    #[tokio::test]
    async fn forwards_lines_with_widget_prefix() {
        let buf = crate::test_support::LogCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_ansi(false).with_writer(buf.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);

        forward_widget_output(&b"first line\nsecond line\n"[..], "weather", "a1b2c3d4", 42).await;

        let output = buf.text();
        assert!(output.contains("weather[a1b2c3d4/42]: first line"));
        assert!(output.contains("weather[a1b2c3d4/42]: second line"));
    }

    #[tokio::test]
    async fn emits_final_line_without_trailing_newline() {
        let buf = crate::test_support::LogCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_ansi(false).with_writer(buf.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);

        forward_widget_output(&b"tail without newline"[..], "weather", "a1b2c3d4", 7).await;

        let output = buf.text();
        assert!(output.contains("weather[a1b2c3d4/7]: tail without newline"));
    }
}
