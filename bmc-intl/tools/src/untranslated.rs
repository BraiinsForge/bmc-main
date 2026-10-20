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

//! `intl untranslated`: the messages still waiting for a translation.

use std::path::Path;
use std::process::ExitCode;

use crate::console;
use crate::surface::{self, Surface, SurfaceName, git};
use crate::translator::{self, Entries};

/// The `--since` that stands for the merge request's base.
pub const MERGE_BASE: &str = "merge-base";

/// List the messages still waiting for a translation, with `since` only those
/// added or reworded after it, any of which is exit 1: the merge request's warning.
///
/// # Errors
/// A `since` that is not a commit, no merge base, or a surface that cannot be read.
pub fn run(surfaces: &[SurfaceName], since: Option<&str>, repo: &Path) -> Result<ExitCode, String> {
    let since = match since {
        Some(MERGE_BASE) => Some(merge_base(repo)?),
        other => other.map(str::to_owned),
    };
    if let Some(rev) = &since
        && !is_commit(repo, rev)
    {
        return Err(format!(
            "invalid value {} for {}: not a commit in {}",
            console::refused(rev),
            console::option("--since"),
            console::path(repo)
        ));
    }
    let mut found = 0;
    for surface in surface::select(surfaces, repo)? {
        let rows = waiting(&surface, since.as_deref().map(|rev| (repo, rev)))?;
        if !rows.is_empty() {
            render(&surface.name, &rows);
            found += rows.len();
        }
    }
    if found == 0 {
        let since_text = since
            .as_deref()
            .map(|rev| format!(" since {}", console::lit(rev)));
        console::ok(&format!(
            "nothing untranslated{}",
            since_text.unwrap_or_default()
        ));
    }
    Ok(if found > 0 && since.is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// The merge request's base in CI, else where the branch
/// left master, as `check-binary-lfs.sh` picks it.
///
/// # Errors
/// Git's reason for each ref tried, when none has a common ancestor with `HEAD`.
pub fn merge_base(repo: &Path) -> Result<String, String> {
    merge_base_given(repo, std::env::var("CI_MERGE_REQUEST_DIFF_BASE_SHA").ok())
}

fn merge_base_given(repo: &Path, ci_base: Option<String>) -> Result<String, String> {
    if let Some(base) = ci_base.filter(|base| !base.is_empty()) {
        return Ok(base);
    }
    let mut failures = Vec::new();
    for target in ["origin/master", "master"] {
        let output = git(repo, &["merge-base", "HEAD", target])?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        failures.push(format!(
            "{target}: {}",
            if reason.is_empty() {
                "no common ancestor"
            } else {
                &reason
            }
        ));
    }
    Err(format!(
        "no merge base in {}, the ref missing or the history unrelated:\n{}",
        console::path(repo),
        failures.join("\n")
    ))
}

fn is_commit(repo: &Path, rev: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .is_ok_and(|output| output.status.success())
}

/// The translator file as `rev` had it, empty when it had none.
fn at_commit(repo: &Path, rev: &str, path: &Path) -> Result<Entries, String> {
    let spec = format!("{rev}:{}", surface::relative(repo, path).display());
    let output = git(repo, &["show", &spec])?;
    if !output.status.success() {
        return Ok(Entries::new());
    }
    translator::decode_entries(&output.stdout, &spec)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub language: String,
    pub id: String,
    pub english: String,
}

/// The surface's untranslated entries, file by file; with `since`
/// (a checkout and a commit of it), less those already waiting
/// there with the same English.
///
/// # Errors
/// A translator file that cannot be read, now or at `since`.
pub fn waiting(surface: &Surface, since: Option<(&Path, &str)>) -> Result<Vec<Waiting>, String> {
    let mut found = Vec::new();
    for path in surface.translator_files()? {
        let entries = translator::read_entries(&path)?;
        let before = match since {
            Some((repo, rev)) => at_commit(repo, rev, &path)?,
            None => Entries::new(),
        };
        let language = surface::language_of(&path);
        found.extend(
            translator::untranslated(&entries)
                .filter(|(id, entry)| {
                    before.get(*id).is_none_or(|before| {
                        before.translation.is_some() || before.source != entry.source
                    })
                })
                .map(|(id, entry)| Waiting {
                    language: language.clone(),
                    id: id.clone(),
                    english: entry.source.clone(),
                }),
        );
    }
    Ok(found)
}

/// A header per surface over a table of its untranslated messages:
/// the language, the English, the id to find it by.
pub fn render(surface: &str, rows: &[Waiting]) {
    console::header(surface);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.language.clone(),
                console::lit(&row.english),
                console::dim(&row.id),
            ]
        })
        .collect();
    console::table(&["language", "English", "id"], &cells);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::translator::{Entry, write_json};

    fn write_translator_file(path: &Path, entries: &[(&str, &str, Option<&str>)]) {
        let entries: Entries = entries
            .iter()
            .map(|(id, source, translation)| ((*id).to_owned(), Entry::new(source, *translation)))
            .collect();
        write_json(path, &entries).expect("BUG: a writable temporary directory");
    }

    fn probe_surface(root: &Path) -> Surface {
        surface::probe_surface(root, "src/**/*.rs")
    }

    /// `git` in `repo` with a committer of its own, so no global config is needed.
    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = git(
            repo,
            &[&["-c", "user.name=t", "-c", "user.email=t@t"], args].concat(),
        )
        .expect("BUG: git on PATH");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn every_null_is_waiting_in_its_language() {
        let (_dir, root) = repo();
        write_translator_file(
            &root.join("lang/zh.json"),
            &[("aaa", "Clear", Some("晴")), ("bbb", "Loading…", None)],
        );
        assert_eq!(
            waiting(&probe_surface(&root), None),
            Ok(vec![Waiting {
                language: "zh".to_owned(),
                id: "bbb".to_owned(),
                english: "Loading…".to_owned(),
            }])
        );
    }

    #[test]
    fn since_a_commit_only_what_it_added_or_reworded_is_waiting() {
        let (_dir, root) = repo();
        run_git(&root, &["init", "-q"]);
        let translator_file = root.join("lang/zh.json");
        write_translator_file(
            &translator_file,
            &[
                ("old", "Clear", None),
                ("reworded", "Probe", None),
                ("done", "Hi", Some("你好")),
            ],
        );
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-q", "-m", "base"]);
        write_translator_file(
            &translator_file,
            &[
                ("old", "Clear", None),
                ("reworded", "Probe 2", None),
                ("done", "Hi", None),
                ("new", "Rain", None),
            ],
        );
        let found =
            waiting(&probe_surface(&root), Some((&root, "HEAD"))).expect("BUG: readable files");
        let ids: Vec<&str> = found.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            ["done", "new", "reworded"],
            "what was already waiting with the same English is not this change"
        );
    }

    #[test]
    fn the_merge_base_is_the_merge_requests_in_ci() {
        assert_eq!(
            merge_base_given(Path::new("."), Some("abc123".to_owned())),
            Ok("abc123".to_owned())
        );
    }

    #[test]
    fn the_merge_base_is_where_the_branch_left_master() {
        let (_dir, root) = repo();
        run_git(&root, &["init", "-q", "-b", "master"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "base"]);
        let base = run_git(&root, &["rev-parse", "HEAD"]);
        run_git(&root, &["switch", "-q", "-c", "topic"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "topic"]);
        assert_eq!(merge_base_given(&root, None), Ok(base));
    }

    #[test]
    fn origins_master_wins_over_a_stale_local_one() {
        let (_dir, root) = repo();
        run_git(&root, &["init", "-q", "-b", "master"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "stale"]);
        run_git(&root, &["switch", "-q", "-c", "topic"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "fetched"]);
        let fetched = run_git(&root, &["rev-parse", "HEAD"]);
        run_git(
            &root,
            &["update-ref", "refs/remotes/origin/master", &fetched],
        );
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "topic"]);
        assert_eq!(merge_base_given(&root, None), Ok(fetched));
    }

    #[test]
    fn no_master_at_all_is_named_with_gits_reason() {
        let (_dir, root) = repo();
        run_git(&root, &["init", "-q", "-b", "main"]);
        run_git(&root, &["commit", "-q", "--allow-empty", "-m", "only main"]);
        let refused = merge_base_given(&root, None);
        assert!(
            refused.as_ref().is_err_and(|error| error
                .contains("the ref missing or the history unrelated")
                && error.contains("origin/master: ")),
            "{refused:?}"
        );
    }
}
