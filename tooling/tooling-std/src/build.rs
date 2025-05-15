// Copyright (C) 2025  Braiins Systems s.r.o.

use chrono::{DateTime, Local, NaiveDateTime, Utc};
use fs_err as fs;
use std::env;
use std::path::PathBuf;

#[macro_export]
macro_rules! include_asset {
    ($name:expr) => {{
        const DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/build-assets/", $name));
        if option_env!(concat!("BUILD_ASSET_", $name, "_LOADED")).is_some() {
            Some(DATA)
        } else {
            None
        }
    }};
}

/// Should be called from `build.rs`.
pub fn copy_asset(src_env: &str) {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("BUG: OUT_DIR is not defined"));

    let dest_dir = out_dir.join("build-assets");
    let dest_path = dest_dir.join(src_env); // yes, we're using the env var as a filename

    println!("cargo:rerun-if-env-changed={src_env}");

    fs::create_dir_all(&dest_dir).expect("failed to create destination directory");

    if let Some(src_path) = env::var_os(src_env) {
        // read+write instead of `fs::copy` to avoid preserving permission bits (for example when copying from the nix store)
        let data = fs::read(src_path).expect("failed to read build asset");
        fs::write(dest_path, data).expect("failed to write build asset");
        println!("cargo:rustc-env=BUILD_ASSET_{src_env}_LOADED=1");
    } else {
        fs::write(&dest_path, "dummy-build-asset").expect("failed to create dummy build asset");
    }
}

pub fn set_git_timestamp() {
    let build_timestamp =
        option_env!("GIT_TIMESTAMP").map_or("unknown".to_owned(), |timestamp_secs| {
            let timestamp_secs = timestamp_secs
                .parse::<i64>()
                .expect("BUG: failed to parse timestamp_secs");

            let naive = NaiveDateTime::from_timestamp_opt(timestamp_secs, 0)
                .expect("BUG: failed to create NaiveDateTime from timestamp_secs");

            let utc: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive, Utc);
            DateTime::<Local>::from(utc).to_rfc3339()
        });

    println!("cargo:rustc-env=GIT_TIMESTAMP={build_timestamp}");
}

pub fn set_git_hash() {
    let commit_hash = option_env!("GIT_HASH").unwrap_or("unknown");
    println!("cargo:rustc-env=GIT_HASH={commit_hash}");
}
