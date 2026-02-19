// Copyright (C) 2026  Braiins Systems s.r.o.

use std::process::Command;

fn main() {
    // Embed git revision at build time (e.g. "fb9af52" or "fb9af52-dirty").
    let git_version = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_owned());

    println!("cargo::rustc-env=GIT_VERSION={git_version}");

    // Rebuild when git HEAD changes (new commits, checkout, etc.).
    println!("cargo::rerun-if-changed=../../.git/HEAD");
    println!("cargo::rerun-if-changed=../../.git/index");
}
