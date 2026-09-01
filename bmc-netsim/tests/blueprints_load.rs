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

//! Every blueprint committed to the repo has to parse.
//!
//! A widget's `sim-blueprint.json5` is otherwise only read
//! when someone starts the simulator, so a mistyped param key
//! sits there until it wastes a recording session.

use std::fs;
use std::path::{Path, PathBuf};

use bmc_netsim::blueprint::Blueprint;

/// Blueprints ship in two places: the simulator's own,
/// and one per widget that drives its scenarios from a fleet.
fn blueprint_paths(repo: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(repo.join("bmc-netsim/blueprints"))
        .expect("BUG: the simulator ships a blueprints directory")
        .map(|entry| entry.expect("BUG: dir entry read failed").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json5"))
        .collect();

    let widgets = fs::read_dir(repo.join("widgets-wasm"))
        .expect("BUG: the widget workspace exists")
        .map(|entry| {
            entry
                .expect("BUG: dir entry read failed")
                .path()
                .join("sim-blueprint.json5")
        })
        .filter(|path| path.is_file());
    paths.extend(widgets);

    paths.sort();
    paths
}

#[test]
fn every_committed_blueprint_parses() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-netsim has a parent repo dir");

    let paths = blueprint_paths(repo);
    assert!(
        paths.len() >= 4,
        "expected the shipped blueprints and at least one widget's, found {paths:?}"
    );

    for path in paths {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("BUG: cannot read {}: {e}", path.display()));
        if let Err(err) = json5::from_str::<Blueprint>(&text) {
            panic!("{} does not parse: {err}", path.display());
        }
    }
}
