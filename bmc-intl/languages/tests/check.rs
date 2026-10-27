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

use std::path::Path;
use std::process::{Command, Output};

/// `bmc-languages check ROOTS`, the firmware's own packs being the roots a device reads.
fn check(roots: &[&Path]) -> Output {
    let profile = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/intl/profile-languages");
    Command::new(env!("CARGO_BIN_EXE_bmc-languages"))
        .arg("check")
        .args(roots)
        .env(bmc_intl::packs::ROOTS_VAR, profile)
        .output()
        .expect("BUG: the binary under test runs")
}

#[test]
fn the_firmware_s_own_packs_check_clean() {
    let output = check(&[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "✓ languages: en, zh"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn a_root_that_is_no_directory_fails_the_check_and_is_named() {
    let root = tempfile::tempdir().expect("BUG: a temporary directory");
    let missing = root.path().join("missing");

    let output = check(&[&missing]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let findings = String::from_utf8_lossy(&output.stderr);
    assert!(
        findings.contains("missing: no directory of packs"),
        "{findings}"
    );
}

#[test]
fn a_pack_with_a_broken_entry_fails_the_check_and_is_named() {
    let root = tempfile::tempdir().expect("BUG: a temporary directory");
    let pack = root.path().join("zh");
    std::fs::create_dir(&pack).expect("BUG: a writable temporary directory");
    std::fs::write(
        pack.join("system.json"),
        r#"{ "broken": { "source": "Connect to {ssid}", "translation": "连接到 {ssid" } }"#,
    )
    .expect("BUG: a writable temporary directory");

    let output = check(&[root.path()]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "✓ languages: en, zh",
        "the broken entry is left out, its language is not"
    );
    let findings = String::from_utf8_lossy(&output.stderr);
    assert!(
        findings.contains("✗ 1 left out")
            && findings.contains(" invalid\n")
            && findings.contains("`broken`"),
        "{findings}"
    );
}
