// Copyright (C) 2026  Braiins Systems s.r.o.

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
