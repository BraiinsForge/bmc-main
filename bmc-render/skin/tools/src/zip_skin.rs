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

//! Pack a skin directory into a zip archive.

use std::path::PathBuf;

use clap::Parser;
use zip::write::SimpleFileOptions;

#[derive(Parser)]
#[command(name = "skin-zip", about = "Pack a skin directory into a zip file")]
struct Args {
    /// Path to the skin directory
    dir: PathBuf,
    /// Output zip file path (default: <dir-name>.zip)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    let dir = &args.dir;

    if !dir.is_dir() {
        eprintln!("error: not a directory: {}", dir.display());
        std::process::exit(1);
    }

    let out = args.output.unwrap_or_else(|| {
        let name = dir
            .file_name()
            .expect("BUG: directory has no name")
            .to_string_lossy();
        PathBuf::from(format!("{name}.zip"))
    });

    let file = std::fs::File::create(&out).unwrap_or_else(|e| {
        eprintln!("error: failed to create {}: {e}", out.display());
        std::process::exit(1);
    });

    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to read directory: {e}");
            std::process::exit(1);
        })
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut count = 0;
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let data = std::fs::read(entry.path()).unwrap_or_else(|e| {
            eprintln!("error: failed to read {name}: {e}");
            std::process::exit(1);
        });
        zip.start_file(&name, options).unwrap_or_else(|e| {
            eprintln!("error: failed to write {name} to zip: {e}");
            std::process::exit(1);
        });
        std::io::Write::write_all(&mut zip, &data).unwrap_or_else(|e| {
            eprintln!("error: failed to write {name} data: {e}");
            std::process::exit(1);
        });
        count += 1;
    }

    zip.finish().unwrap_or_else(|e| {
        eprintln!("error: failed to finalize zip: {e}");
        std::process::exit(1);
    });

    eprintln!("wrote {count} files to {}", out.display());
}
