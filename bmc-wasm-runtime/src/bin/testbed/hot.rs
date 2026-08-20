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

//! The rebuild cycle: watching the widget's source, building it, and saying
//! where that got to.
//!
//! The testbed runs cargo itself rather than watching for someone else to.
//! An operator editing a widget wants the preview to follow the edit, and
//! only the side that starts the build can say whether it is running, how
//! long it has taken, or why it failed — a file appearing on disk says none
//! of that, and says nothing at all when the build never got that far.

use std::io::{BufRead as _, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

/// How long a finished cycle stays on the chip before it lapses to watching.
const LINGER: Duration = Duration::from_secs(4);

/// How long to wait for the built wasm to be picked up before giving up on it.
/// A build can compile nothing and leave the file alone, which never arrives.
const SWAP_WAIT: Duration = Duration::from_millis(1_500);

/// The quiet before a build: an editor writes a file in several steps, and
/// "save all" touches many.
const QUIET: Duration = Duration::from_millis(500);

/// Where the cycle has got to.
#[derive(Clone, Debug)]
pub(crate) enum HotPhase {
    /// Nothing to do; an edit would start something.
    Watching,
    /// An edit landed, and the quiet before the build is running.
    Changed,
    /// Cargo is building; the chip counts from `since`.
    Building { since: Instant },
    /// The build came out; its wasm is not in the runtimes yet.
    Swapping { since: Instant },
    /// What is on screen is the edit's.
    Reloaded { at: Instant, took: Duration },
    /// The build failed, and everything it said.
    Failed(Arc<BuildFailure>),
    /// Nothing is watching any more, and why.
    Stopped { why: String },
}

/// A failed build: everything it said, and how many errors that came to.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct BuildFailure {
    /// Every message rustc rendered.
    pub(crate) messages: Vec<BuildMessage>,
    /// Errors with a site of their own; zero where nothing rendered a
    /// diagnostic, as an unparseable manifest does — which is why the bar
    /// reads the count rather than stating it.
    pub(crate) errors: usize,
}

/// One message rustc rendered, as the window shows it.
#[derive(Debug, PartialEq)]
pub(crate) struct BuildMessage {
    pub(crate) level: MessageLevel,
    pub(crate) text: String,
}

/// What a message weighs; anything rustc says that is neither is a note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MessageLevel {
    Error,
    Warning,
    Note,
}

/// The rebuild cycle as the window reads it: one handle, cloned to each side
/// that reports into it.
///
/// The phase is only ever moved through the ops here, so a cycle cannot be
/// left half-advanced by a caller that forgot a step.
#[derive(Clone)]
pub(crate) struct HotStatus(Arc<Mutex<HotState>>);

/// What the handle shares: the phase, and the window to wake when it moves.
struct HotState {
    phase: HotPhase,
    /// Taken from the first frame, so the watcher's thread can wake a window
    /// that is otherwise asleep.
    ctx: Option<egui::Context>,
}

impl HotStatus {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(HotState {
            phase: HotPhase::Watching,
            ctx: None,
        })))
    }

    /// The phase to draw.
    pub(crate) fn phase(&self) -> HotPhase {
        self.state().phase.clone()
    }

    /// Hand over the window to wake. Called every frame; the first lands.
    pub(crate) fn wake_with(&self, ctx: &egui::Context) {
        let mut state = self.state();
        if state.ctx.is_none() {
            state.ctx = Some(ctx.clone());
        }
    }

    /// The wasm was reloaded — what the views run is what was just built.
    pub(crate) fn swapped(&self) {
        let mut state = self.state();
        let took = match state.phase {
            HotPhase::Swapping { since } => since.elapsed(),
            // A build run elsewhere wrote the wasm; it reloaded all the same.
            HotPhase::Watching
            | HotPhase::Changed
            | HotPhase::Building { .. }
            | HotPhase::Reloaded { .. }
            | HotPhase::Failed(_)
            | HotPhase::Stopped { .. } => Duration::ZERO,
        };
        state.phase = HotPhase::Reloaded {
            at: Instant::now(),
            took,
        };
    }

    /// Let a finished cycle lapse back to watching, and give up on a swap
    /// that is not coming.
    pub(crate) fn settle(&self) {
        let mut state = self.state();
        let lapsed = match state.phase {
            HotPhase::Reloaded { at, .. } => at.elapsed() >= LINGER,
            HotPhase::Swapping { since } => since.elapsed() >= SWAP_WAIT,
            HotPhase::Watching
            | HotPhase::Changed
            | HotPhase::Building { .. }
            | HotPhase::Failed(_)
            | HotPhase::Stopped { .. } => false,
        };
        if lapsed {
            state.phase = HotPhase::Watching;
        }
    }

    /// Move to `phase` and wake the window, which is how a build starting
    /// shows up on a canvas at rest.
    fn set(&self, phase: HotPhase) {
        // The window is woken with the phase lock released: waking takes locks
        // of egui's own, and this runs on the watcher's thread while the UI
        // thread is reading the phase.
        let ctx = {
            let mut state = self.state();
            state.phase = phase;
            state.ctx.clone()
        };
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, HotState> {
        self.0.lock().expect("BUG: the hot phase is poisoned")
    }
}

#[cfg(test)]
impl HotStatus {
    /// A cycle held at `phase`, for a test with no cargo to run.
    pub(crate) fn posed(phase: HotPhase) -> Self {
        let hot = Self::new();
        hot.set(phase);
        hot
    }
}

/// A running watcher: the build it has in flight, and whether to keep going.
#[derive(Clone)]
pub(crate) struct SourceWatcher {
    building: Arc<Mutex<Option<std::process::Child>>>,
    stopped: Arc<AtomicBool>,
}

impl SourceWatcher {
    /// Stop watching and take down any build in flight. The thread is left to
    /// the process exit that follows: it is blocked on the channel or inside
    /// the build this just killed.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(building) = self
            .building
            .lock()
            .expect("BUG: the build in flight is poisoned")
            .as_mut()
        {
            let _ = building.kill();
        }
    }

    fn going(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }
}

/// Watch `widget_root` and rebuild its wasm as edits land, reporting the
/// cycle into `hot`.
///
/// `workspace` is the directory cargo runs in — the widget's workspace root,
/// which is where its lock file and `target/` live.
pub(crate) fn spawn(widget_root: PathBuf, workspace: PathBuf, hot: &HotStatus) -> SourceWatcher {
    let watcher = SourceWatcher {
        building: Arc::default(),
        stopped: Arc::new(AtomicBool::new(false)),
    };
    let running = watcher.clone();
    let reporting = hot.clone();
    thread::spawn(move || {
        if let Err(why) = watch(&widget_root, &workspace, &reporting, &running) {
            reporting.set(HotPhase::Stopped { why });
        }
    });
    watcher
}

/// Set the watch up, then run the loop over what it reports.
fn watch(
    widget_root: &Path,
    workspace: &Path,
    hot: &HotStatus,
    watcher: &SourceWatcher,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let mut notify =
        notify::recommended_watcher(tx).map_err(|e| format!("no file watcher: {e}"))?;
    notify
        .watch(widget_root, RecursiveMode::Recursive)
        .map_err(|e| format!("cannot watch `{}`: {e}", widget_root.display()))?;

    // Wait for an edit, wait for the edits to stop, rebuild, and go round
    // again. What the build writes comes back through the same channel and is
    // dropped by the filter, so a build never triggers itself.
    while watcher.going() {
        let Ok(event) = rx.recv() else {
            return Ok(());
        };
        let mut edited = is_edit(&event);

        let mut quiet = Instant::now() + QUIET;
        while let Some(wait) = quiet.checked_duration_since(Instant::now()) {
            if edited {
                hot.set(HotPhase::Changed);
            }
            match rx.recv_timeout(wait) {
                Ok(event) => {
                    if is_edit(&event) {
                        edited = true;
                        quiet = Instant::now() + QUIET;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        if edited && watcher.going() {
            build(widget_root, workspace, hot, watcher);
        }
    }
    Ok(())
}

/// Whether an event is an edit worth rebuilding for.
fn is_edit(event: &notify::Result<notify::Event>) -> bool {
    let Ok(event) = event else {
        return false;
    };
    if !matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|path| is_source(path))
}

/// What the build itself writes is not an edit, nor is what the repository
/// keeps: without this the first rebuild triggers the next, forever.
fn is_source(path: &Path) -> bool {
    !path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | ".git" | ".cache" | ".tmp")
        )
    })
}

/// Build the widget's wasm, reporting what cargo said.
fn build(widget_root: &Path, workspace: &Path, hot: &HotStatus, watcher: &SourceWatcher) {
    hot.set(HotPhase::Building {
        since: Instant::now(),
    });

    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .arg("build")
        .arg("--manifest-path")
        .arg(widget_root.join("Cargo.toml"))
        .arg("--target")
        .arg(WASM_TARGET)
        // Records on stdout, each diagnostic rendered by rustc with the SGR
        // codes it would print to a terminal: the window colours the report
        // from rustc's own markup rather than guessing at the text.
        .arg("--message-format=json-diagnostic-rendered-ansi")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            hot.set(HotPhase::Stopped {
                why: format!("cargo did not start: {e}"),
            });
            return;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *watcher
        .building
        .lock()
        .expect("BUG: the build in flight is poisoned") = Some(child);

    // Cargo writes to both, and a pipe nobody is reading fills up and blocks it.
    let tail = stderr.map(|stderr| thread::spawn(move || forward_output(stderr)));
    let report = stdout.map(BuildReport::read).unwrap_or_default();
    let tail = tail.and_then(|pump| pump.join().ok()).unwrap_or_default();

    // Taken out before it is waited on: holding the lock across the wait
    // leaves `stop` blocking on the child it is killing.
    let child = watcher
        .building
        .lock()
        .expect("BUG: the build in flight is poisoned")
        .take();
    if let Some(mut child) = child {
        let _ = child.wait();
    }

    // A build stopped on the way out is not a failure to report.
    if watcher.going() {
        hot.set(report.came_to(tail));
    }
}

const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Lines of cargo's own output kept back, for a failure that renders no
/// diagnostics — an unparseable manifest says everything on stderr.
const TAIL_LINES: usize = 20;

/// What a build said, gathered as it says it.
#[derive(Debug, Default, PartialEq)]
struct BuildReport {
    messages: Vec<BuildMessage>,
    errors: usize,
    /// Whether anything was actually compiled. Cargo marks what it did not
    /// rebuild as fresh, and a build of nothing but those leaves the wasm
    /// untouched — so nothing will arrive to swap.
    compiled: bool,
    outcome: Option<BuildOutcome>,
}

/// How cargo said the build ended.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BuildOutcome {
    Built,
    Failed,
}

/// One line of cargo's stream. Nothing else it emits is read.
#[derive(serde::Deserialize)]
#[serde(tag = "reason")]
enum CargoRecord {
    #[serde(rename = "compiler-message")]
    CompilerMessage { message: CargoMessage },
    #[serde(rename = "compiler-artifact")]
    CompilerArtifact { fresh: bool },
    #[serde(rename = "build-finished")]
    BuildFinished { success: bool },
    #[serde(other)]
    Other,
}

/// One rustc message as cargo's record carries it, as much of it as is read.
#[derive(serde::Deserialize)]
struct CargoMessage {
    level: String,
    /// Absent for a message rustc did not render, which is nothing to show.
    rendered: Option<String>,
    /// A summary — "aborting due to 2 previous errors" — carries no code and
    /// no span, and is no error to count.
    code: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    spans: Vec<serde::de::IgnoredAny>,
}

impl BuildReport {
    /// Read cargo's records to the end, putting each rendered message on the
    /// terminal as it lands.
    fn read(stream: impl Read) -> Self {
        let mut report = Self::default();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Some(rendered) = report.take(&line) {
                eprint!("{rendered}");
            }
        }
        report
    }

    /// Fold one of cargo's lines in, returning what it rendered for the
    /// terminal.
    fn take(&mut self, line: &str) -> Option<String> {
        match serde_json::from_str::<CargoRecord>(line).ok()? {
            CargoRecord::CompilerMessage { message } => {
                let rendered = message.rendered?;
                let level = match message.level.as_str() {
                    "error" | "error: internal compiler error" => MessageLevel::Error,
                    "warning" => MessageLevel::Warning,
                    _ => MessageLevel::Note,
                };
                // A summary line counts no error of its own: it is the count.
                if level == MessageLevel::Error
                    && (message.code.is_some() || !message.spans.is_empty())
                {
                    self.errors += 1;
                }
                self.messages.push(BuildMessage {
                    level,
                    text: rendered.clone(),
                });
                Some(rendered)
            }
            CargoRecord::CompilerArtifact { fresh } => {
                self.compiled |= !fresh;
                None
            }
            CargoRecord::BuildFinished { success } => {
                self.outcome = Some(if success {
                    BuildOutcome::Built
                } else {
                    BuildOutcome::Failed
                });
                None
            }
            CargoRecord::Other => None,
        }
    }

    /// The phase this build ended in. `tail` is cargo's own output, for a
    /// failure that rendered no diagnostics of its own.
    fn came_to(self, tail: String) -> HotPhase {
        match self.outcome {
            Some(BuildOutcome::Built) if self.compiled => HotPhase::Swapping {
                since: Instant::now(),
            },
            // Nothing was rebuilt, so no wasm will be written and nothing
            // will arrive to swap. The cycle is over where it stands.
            Some(BuildOutcome::Built) => HotPhase::Watching,
            _ => {
                let mut failure = BuildFailure {
                    errors: self.errors,
                    messages: self.messages,
                };
                if failure.messages.is_empty() && !tail.is_empty() {
                    failure.messages.push(BuildMessage {
                        level: MessageLevel::Error,
                        text: tail,
                    });
                }
                HotPhase::Failed(Arc::new(failure))
            }
        }
    }
}

/// Put cargo's own output on the terminal, keeping the tail for a failure
/// that rendered no diagnostics.
fn forward_output(stream: impl Read) -> String {
    let mut tail = std::collections::VecDeque::with_capacity(TAIL_LINES);
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        eprintln!("{line}");
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    Vec::from(tail).join("\n")
}

#[cfg(test)]
mod tests {
    use super::{BuildOutcome, BuildReport, HotPhase, HotStatus, MessageLevel};

    /// A cargo record for a rendered message at `level`, with a code where
    /// one is wanted — which is what marks an error with a site of its own.
    fn message(level: &str, rendered: &str, coded: bool) -> String {
        let code = if coded { r#""E0425""# } else { "null" };
        format!(
            r#"{{"reason":"compiler-message","message":{{"level":"{level}","rendered":"{rendered}","code":{code},"spans":[]}}}}"#
        )
    }

    #[test]
    fn a_summary_line_is_shown_but_counts_no_error_of_its_own() {
        let mut report = BuildReport::default();
        report.take(&message("error", "cannot find value `x`", true));
        report.take(&message("error", "aborting due to 1 previous error", false));

        assert_eq!(
            report.errors, 1,
            "the summary restates the count rather than adding to it",
        );
        assert_eq!(report.messages.len(), 2, "both are still shown");
    }

    #[test]
    fn a_warning_is_kept_without_counting_as_a_failure() {
        let mut report = BuildReport::default();
        report.take(&message("warning", "unused variable `y`", true));

        assert_eq!(report.errors, 0);
        assert_eq!(report.messages[0].level, MessageLevel::Warning);
    }

    #[test]
    fn a_build_that_compiled_nothing_expects_no_swap() {
        let mut report = BuildReport::default();
        report.take(r#"{"reason":"compiler-artifact","fresh":true}"#);
        report.take(r#"{"reason":"build-finished","success":true}"#);

        assert_eq!(report.outcome, Some(BuildOutcome::Built));
        assert!(
            matches!(report.came_to(String::new()), HotPhase::Watching),
            "nothing was rebuilt, so no wasm will arrive to swap",
        );
    }

    #[test]
    fn a_build_that_compiled_waits_for_the_wasm() {
        let mut report = BuildReport::default();
        report.take(r#"{"reason":"compiler-artifact","fresh":false}"#);
        report.take(r#"{"reason":"build-finished","success":true}"#);

        assert!(matches!(
            report.came_to(String::new()),
            HotPhase::Swapping { .. }
        ));
    }

    #[test]
    fn a_failure_with_nothing_rendered_falls_back_to_what_cargo_said() {
        let mut report = BuildReport::default();
        report.take(r#"{"reason":"build-finished","success":false}"#);

        let HotPhase::Failed(failure) = report.came_to("error: no such manifest".to_owned()) else {
            panic!("BUG: a failed build must report a failure");
        };
        assert_eq!(failure.errors, 0, "nothing rendered a diagnostic to count");
        assert_eq!(
            failure.messages[0].text, "error: no such manifest",
            "cargo's own words are all there is to show",
        );
    }

    #[test]
    fn a_swap_that_never_arrives_lapses_back_to_watching() {
        let hot = HotStatus::posed(HotPhase::Swapping {
            since: std::time::Instant::now()
                .checked_sub(super::SWAP_WAIT)
                .expect("BUG: the clock must reach back past one swap wait"),
        });

        hot.settle();

        assert!(
            matches!(hot.phase(), HotPhase::Watching),
            "a build that wrote nothing must not leave the chip mid-cycle",
        );
    }

    #[test]
    fn a_reload_outside_a_build_is_still_reported() {
        let hot = HotStatus::new();

        hot.swapped();

        let HotPhase::Reloaded { took, .. } = hot.phase() else {
            panic!("BUG: a swap must report a reload");
        };
        assert_eq!(
            took,
            std::time::Duration::ZERO,
            "a build run elsewhere has no duration of ours to report",
        );
    }
}
