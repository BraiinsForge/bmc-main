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

//! `bmc-netsim` entry point: run a blueprint of device instances
//! loaded from disk, or emit the blueprint JSON schema for authoring.

use std::fmt;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format::Writer;

use bmc_netsim::blueprint::Blueprint;
use bmc_netsim::diag;

#[derive(Parser, Debug)]
#[command(
    name = "bmc-netsim",
    about = "Generic mDNS + REST network-resource simulator"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Load a blueprint (JSON5) and run the simulated devices on the LAN.
    Run { blueprint: PathBuf },
    /// Print the blueprint JSON schema to stdout.
    Schema,
}

/// Field formatting that tints `key=` so the dense fleet lines scan by eye:
/// the message prints plain, keys cyan, values unquoted. ANSI only when the
/// writer supports it (TTY), so piped output stays clean.
struct KvFields;

struct KvVisitor<'a, 'w> {
    writer: &'a mut Writer<'w>,
    result: fmt::Result,
}

impl KvVisitor<'_, '_> {
    fn record(&mut self, field: &tracing::field::Field, value: &dyn fmt::Display) {
        if self.result.is_err() {
            return;
        }
        self.result = if field.name() == "message" {
            write!(self.writer, "{value}")
        } else if self.writer.has_ansi_escapes() {
            write!(
                self.writer,
                " \x1b[36m{}=\x1b[92m{value}\x1b[0m",
                field.name()
            )
        } else {
            write!(self.writer, " {}={value}", field.name())
        };
    }
}

impl tracing::field::Visit for KvVisitor<'_, '_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.record(field, &format_args!("{value:?}"));
    }

    // Bare, not `Debug`-quoted.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record(field, &value);
    }
}

impl<'w> FormatFields<'w> for KvFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        mut writer: Writer<'w>,
        fields: R,
    ) -> fmt::Result {
        let mut visitor = KvVisitor {
            writer: &mut writer,
            result: Ok(()),
        };
        fields.record(&mut visitor);
        visitor.result
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // The subscriber writes to stdout; colour it only when that is a
        // terminal, so a redirected log holds text rather than escapes.
        .with_ansi(std::io::stdout().is_terminal())
        .fmt_fields(KvFields)
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Schema => {
            let schema = schemars::schema_for!(Blueprint);
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        Command::Run { blueprint } => {
            let text = std::fs::read_to_string(&blueprint)
                .with_context(|| format!("reading {}", blueprint.display()))?;
            let mut parsed = match json5::from_str::<Blueprint>(&text) {
                Ok(parsed) => parsed,
                Err(err) => {
                    diag::emit_error(&blueprint, &text, &err);
                    bail!("invalid blueprint {}", blueprint.display());
                }
            };
            parsed.resolve_paths(blueprint.parent().unwrap_or(Path::new(".")));
            tracing::info!(instances = parsed.instances.len(), "blueprint loaded");
            bmc_netsim::serve(parsed).await?;
        }
    }
    Ok(())
}
