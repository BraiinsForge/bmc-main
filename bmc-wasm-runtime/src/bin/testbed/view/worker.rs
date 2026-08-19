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

//! A view that renders on its own thread.
//!
//! The worker owns its GL context, its renderer and its widget for life, and
//! the compositor only ever learns texture names and a fence.
//!
//! The renderer always draws into one target — the renderer cannot be
//! retargeted per frame (see [`crate::paint::ViewTargets::blit_to`]) — and
//! each finished frame is blitted into one of two present targets, taken in
//! turn. The compositor paints whichever the handoff names, so a present
//! target is rewritten only after another handoff and a paint have passed,
//! never while it is the one on screen.

use std::sync::mpsc;

use super::{ViewCommand, ViewCore, ViewInfo, ViewReport, ViewTick, ViewTicked};
use crate::window::OffscreenContext;

/// A frame the worker has finished drawing.
///
/// It names a slot, not a texture: the compositor registered both present
/// textures at startup and paints whichever this says. Handing a texture over
/// per frame would mean `Painter::replace_native_texture`, which queues
/// whatever it displaces for deletion — and the view owns its textures, not
/// the painter.
struct Handoff {
    /// Which present target holds the frame.
    slot: usize,
    /// Orders the compositor's sampling behind the worker's drawing.
    /// `None` when the fence path is unavailable and the worker drained its
    /// own queue with `glFinish` instead.
    ///
    /// One way only: nothing tells the worker a slot is done being sampled
    /// before it draws there again. GL orders no streams across contexts.
    /// What carries it is slack — a blocking `drive` and vsync leave a frame,
    /// which in-order submission has drained. Out of order, one view tears;
    /// closing it needs a release fence per slot, carried on the next tick.
    fence: Option<FenceHandle>,
}

/// A `GLsync` on its way to the thread that will wait on it.
///
/// Sync objects are share-group state, so waiting on one from the compositor's
/// context is exactly what they are for; only the raw pointer is not `Send`.
struct FenceHandle(*mut std::ffi::c_void);

// SAFETY: a GLsync belongs to the share group, not to the thread that made it.
// The worker creates it and never touches it again; the compositor waits on it
// and deletes it.
unsafe impl Send for FenceHandle {}

enum ToWorker {
    Apply(ViewCommand),
    Tick(ViewTick),
    /// Rebuild the widget from these ingredients, in place.
    Reload {
        seed: Box<super::ViewSeed>,
        rebind: Box<super::Rebind>,
    },
    Close,
}

enum FromWorker {
    Ticked {
        ticked: ViewTicked,
        report: ViewReport,
        handoff: Option<Handoff>,
    },
    /// The rebuild's outcome, carrying the info the new runtime reports —
    /// the SDK version and export set change with the widget.
    Reloaded(Result<ViewInfo, String>),
}

/// What a reload reports when the thread is gone: the rebuild's own failure
/// is the worker's to describe, and this is what is left when none arrives.
const NO_ANSWER: &str = "the view thread stopped answering";

/// The compositor's end of a threaded view.
pub(crate) struct Worker {
    tx: mpsc::Sender<ToWorker>,
    rx: mpsc::Receiver<FromWorker>,
    join: Option<std::thread::JoinHandle<()>>,
    info: ViewInfo,
    report: ViewReport,
    label: String,
    /// Which present target holds the newest frame, and so gets painted.
    showing: usize,
    /// Latches once the thread stops answering, so the view says so instead of
    /// quietly freezing at its last frame.
    lost: bool,
}

impl Worker {
    pub(crate) fn info(&self) -> ViewInfo {
        self.info
    }

    pub(crate) fn report(&self) -> ViewReport {
        self.report.clone()
    }

    /// Which of the two registered textures holds the newest frame.
    pub(crate) fn showing(&self) -> usize {
        self.showing
    }

    /// Hand the worker this tick's commands and wait for the frame it produces.
    ///
    /// Waiting means views tick one after the next, not at the same time.
    /// What the thread buys is ownership: a `!Send` runtime and its GL context
    /// stay on the thread that made them, and tear down there.
    pub(crate) fn drive(
        &mut self,
        tick: &ViewTick,
        commands: Vec<ViewCommand>,
        gl: &egui_glow::glow::Context,
    ) -> ViewTicked {
        for command in commands {
            if self.tx.send(ToWorker::Apply(command)).is_err() {
                return self.lost();
            }
        }
        if self.tx.send(ToWorker::Tick(*tick)).is_err() {
            return self.lost();
        }
        let Ok(FromWorker::Ticked {
            ticked,
            report,
            handoff,
        }) = self.rx.recv()
        else {
            return self.lost();
        };
        self.report = report;

        if let Some(handoff) = handoff {
            Self::wait_for_frame(&handoff, gl);
            self.showing = handoff.slot;
        }
        ticked
    }

    /// Rebuild the widget on the thread that owns its renderer.
    ///
    /// Waits for the answer, because the rebuild can fail and a new widget
    /// reports a new `ViewInfo` — neither is knowable from this side.
    ///
    /// Not per-frame, so the wait costs only a rebuild that has to finish
    /// anyway.
    pub(crate) fn reload(
        &mut self,
        seed: super::ViewSeed,
        rebind: super::Rebind,
    ) -> Result<(), String> {
        if self
            .tx
            .send(ToWorker::Reload {
                seed: Box::new(seed),
                rebind: Box::new(rebind),
            })
            .is_err()
        {
            let _ = self.lost();
            return Err(NO_ANSWER.to_owned());
        }
        self.take_reload_ack()
    }

    /// Read the worker's answer to a reload, adopting the `ViewInfo` a rebuilt
    /// widget reports.
    fn take_reload_ack(&mut self) -> Result<(), String> {
        match self.rx.recv() {
            Ok(FromWorker::Reloaded(Ok(info))) => {
                self.info = info;
                Ok(())
            }
            Ok(FromWorker::Reloaded(Err(e))) => Err(e),
            // A `Ticked` here would mean the protocol stopped alternating.
            Ok(FromWorker::Ticked { .. }) | Err(_) => {
                let _ = self.lost();
                Err(NO_ANSWER.to_owned())
            }
        }
    }

    /// The thread stopped answering. Say so once, then let the view sit on its
    /// last frame — the alternative is a window that silently stops moving.
    fn lost(&mut self) -> ViewTicked {
        if !self.lost {
            self.lost = true;
            tracing::error!(label = %self.label, "view thread stopped answering");
        }
        ViewTicked::default()
    }

    /// Wait for the GPU to agree the frame is finished, before anything on this
    /// side samples the texture holding it.
    fn wait_for_frame(handoff: &Handoff, gl: &egui_glow::glow::Context) {
        let Some(FenceHandle(raw)) = handoff.fence else {
            return;
        };
        let fence = egui_glow::glow::NativeFence(raw.cast());
        // SAFETY: the compositor's context is current on this thread, and the
        // fence was created in its share group.
        unsafe {
            use egui_glow::glow::HasContext as _;
            // Server-side: it orders the GPU's queues without stalling the UI
            // thread, which a client wait would.
            gl.wait_sync(fence, 0, egui_glow::glow::TIMEOUT_IGNORED);
            gl.delete_sync(fence);
        }
    }

    /// Ask the thread to stop, without waiting for it.
    ///
    /// Dropping a runtime blocks until its background I/O winds down, which a
    /// fetch in flight can hold for its whole timeout — so the UI never joins
    /// here. The returned handle is reaped a frame at a time instead.
    pub(crate) fn shutdown(mut self) -> Retired {
        drop(self.tx.send(ToWorker::Close));
        Retired {
            join: self.join.take(),
            label: self.label,
            closed_at: std::time::Instant::now(),
            warned: false,
        }
    }
}

/// A worker asked to stop, on its way out.
///
/// The thread frees its own GL objects before it exits — they belong to its
/// context, so nothing else could — which is why the compositor has nothing to
/// wait *for*: the join only observes that the exit happened, and reports a
/// panic that would otherwise vanish. Dropping this unreaped detaches the
/// thread, which only the end of the process should do.
pub(crate) struct Retired {
    join: Option<std::thread::JoinHandle<()>>,
    label: String,
    closed_at: std::time::Instant,
    warned: bool,
}

/// How long a closing worker may take before it is worth a log line.
/// Generous because a fetch in flight legitimately holds teardown
/// for its I/O timeout; anything past this is worth investigating.
const REAP_WARN_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

impl Retired {
    /// Collect the thread if it has exited; `None` once it is gone.
    ///
    /// Never blocks: a thread still winding down is handed back, and the
    /// caller tries again next frame.
    pub(crate) fn reap(mut self) -> Option<Self> {
        let join = self.join.take()?;
        if !join.is_finished() {
            if !self.warned && self.closed_at.elapsed() > REAP_WARN_AFTER {
                self.warned = true;
                tracing::warn!(
                    label = %self.label,
                    "view worker still shutting down after {}s",
                    REAP_WARN_AFTER.as_secs()
                );
            }
            self.join = Some(join);
            return Some(self);
        }
        if join.join().is_err() {
            tracing::warn!(label = %self.label, "view worker panicked on the way out");
        }
        None
    }

    /// Wait for the thread, however long it takes.
    ///
    /// For process exit only, where there is no UI left to stall and a
    /// detached worker would race its GL context against the dying display
    /// connection.
    pub(crate) fn reap_blocking(self) {
        if let Some(join) = self.join
            && join.join().is_err()
        {
            tracing::warn!(label = %self.label, "view worker panicked on the way out");
        }
    }
}

/// Everything the worker needs to build its own half, all of it `Send`.
pub(crate) struct WorkerSeed {
    pub(crate) offscreen: OffscreenContext,
    pub(crate) seed: super::ViewSeed,
    pub(crate) label: String,
    pub(crate) led_rx: Option<mpsc::Receiver<bmc_wasm_runtime::LedRequest>>,
    /// Overrides what the GL version would choose.
    pub(crate) handoff: Option<super::fence::GpuWait>,
}

/// Start a view on its own thread, or fail before anything is spawned.
///
/// Returns the worker and the names of its two present textures, which the
/// compositor registers once and then paints in whichever order the handoffs
/// dictate.
pub(crate) fn spawn(seed: WorkerSeed) -> anyhow::Result<(Worker, [u32; 2])> {
    let (to_worker, from_ui) = mpsc::channel();
    let (to_ui, from_worker) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let label = seed.label.clone();

    let join = std::thread::Builder::new()
        .name(format!("view-{label}"))
        .spawn(move || run(seed, &from_ui, &to_ui, &ready_tx))
        .map_err(|e| anyhow::anyhow!("spawn view thread: {e}"))?;

    // The build happens on the worker, so its failure arrives as a message
    // rather than a return: nothing else can construct a `!Send` renderer.
    match ready_rx.recv() {
        Ok(Ok(ready)) => {
            tracing::info!(
                label,
                handoff = ?ready.gpu_wait,
                "view: rendering on its own thread"
            );
            Ok((
                Worker {
                    tx: to_worker,
                    rx: from_worker,
                    join: Some(join),
                    info: ready.info,
                    report: ViewReport::default(),
                    label: label.clone(),
                    showing: 0,
                    lost: false,
                },
                ready.textures,
            ))
        }
        Ok(Err(e)) => {
            drop(join.join());
            Err(anyhow::anyhow!("{e}"))
        }
        Err(_) => {
            drop(join.join());
            Err(anyhow::anyhow!("view thread died before it reported"))
        }
    }
}

struct Ready {
    info: ViewInfo,
    textures: [u32; 2],
    gpu_wait: super::fence::GpuWait,
}

/// The worker thread: build in place, then serve the compositor until closed.
fn run(
    seed: WorkerSeed,
    from_ui: &mpsc::Receiver<ToWorker>,
    to_ui: &mpsc::Sender<FromWorker>,
    ready_tx: &mpsc::Sender<Result<Ready, String>>,
) {
    let mut state = match build(seed) {
        Ok(state) => state,
        Err(e) => {
            drop(ready_tx.send(Err(format!("{e:#}"))));
            return;
        }
    };
    if ready_tx
        .send(Ok(Ready {
            info: state.core.info(),
            textures: [
                state.present[0].texture_name(),
                state.present[1].texture_name(),
            ],
            gpu_wait: state.gpu_wait,
        }))
        .is_err()
    {
        return;
    }

    while let Ok(message) = from_ui.recv() {
        match message {
            ToWorker::Apply(command) => state.core.apply(command),
            ToWorker::Reload { seed, rebind } => {
                let reloaded = state.reload(*seed, *rebind);
                if to_ui.send(FromWorker::Reloaded(reloaded)).is_err() {
                    break;
                }
            }
            ToWorker::Close => break,
            ToWorker::Tick(tick) => {
                let ticked = state.render(&tick);
                if to_ui.send(ticked).is_err() {
                    break;
                }
            }
        }
    }
    state.destroy();
}

/// The worker's own half of a view.
struct WorkerState {
    core: ViewCore,
    gl: egui_glow::glow::Context,
    /// What the widget draws into, every frame. Never sampled by the
    /// compositor and never registered with the painter.
    targets: crate::paint::ViewTargets,
    /// What the compositor samples: finished frames blitted out of `targets`,
    /// taken in turn.
    present: [crate::paint::ViewTargets; 2],
    /// The present target the next finished frame is blitted into.
    writing: usize,
    gpu_wait: super::fence::GpuWait,
    /// Latches once a fence could not be made, so a driver that refuses them
    /// says so once rather than once per frame.
    fence_failed: bool,
    /// Kept current for the thread's life; dropping it releases the context.
    _context: glutin::context::PossiblyCurrentContext,
    _surface: glutin::surface::Surface<glutin::surface::PbufferSurface>,
}

fn build(seed: WorkerSeed) -> anyhow::Result<WorkerState> {
    use glutin::context::NotCurrentGlContext as _;
    use glutin::display::{GetGlDisplay as _, GlDisplay as _};

    let OffscreenContext { context, surface } = seed.offscreen;
    let context = context
        .make_current(&surface)
        .map_err(|e| anyhow::anyhow!("make the view's context current: {e}"))?;

    // SAFETY: current on this thread from here until the thread ends.
    let gl = unsafe {
        egui_glow::glow::Context::from_loader_function_cstr(|name| {
            context.display().get_proc_address(name)
        })
    };
    // SAFETY: same.
    let version = unsafe {
        use egui_glow::glow::HasContext as _;
        gl.get_parameter_string(egui_glow::glow::VERSION)
    };
    let gpu_wait = seed
        .handoff
        .unwrap_or_else(|| super::fence::gpu_wait_for_version(&version));

    let (width, height) = (seed.seed.width, seed.seed.height);
    let present = [
        crate::paint::ViewTargets::create(&gl, width, height)?,
        crate::paint::ViewTargets::create(&gl, width, height)?,
    ];
    let parts = seed.seed.build(&gl)?;
    let core = ViewCore::new(
        parts.runtime,
        parts.renderer,
        width,
        height,
        seed.label,
        seed.led_rx,
    );

    Ok(WorkerState {
        core,
        gl,
        targets: parts.targets,
        present,
        writing: 0,
        gpu_wait,
        fence_failed: false,
        _context: context,
        _surface: surface,
    })
}

impl WorkerState {
    /// Run one tick and, if it drew, hand the frame over.
    fn render(&mut self, tick: &ViewTick) -> FromWorker {
        let ticked = self.core.tick(tick);
        let report = self.core.report();
        let handoff = ticked.rendered.then(|| {
            let slot = self.writing;
            self.writing = 1 - slot;
            self.targets.blit_to(&self.gl, &self.present[slot]);
            Handoff {
                slot,
                fence: self.seal(),
            }
        });
        FromWorker::Ticked {
            ticked,
            report,
            handoff,
        }
    }

    /// Make this thread's drawing visible to whoever samples the texture next.
    fn seal(&mut self) -> Option<FenceHandle> {
        use egui_glow::glow::HasContext as _;

        // SAFETY: the worker's context is current on this thread.
        unsafe {
            match self.gpu_wait {
                super::fence::GpuWait::FenceSync => {
                    // `None` says the frame is already synchronised,
                    // so a fence that could not be made has to synchronise first.
                    let fence = self
                        .gl
                        .fence_sync(egui_glow::glow::SYNC_GPU_COMMANDS_COMPLETE, 0);
                    let Ok(fence) = fence else {
                        if !self.fence_failed {
                            self.fence_failed = true;
                            tracing::warn!(
                                label = %self.core.label,
                                "fence creation failed; draining the queue instead"
                            );
                        }
                        self.gl.finish();
                        return None;
                    };
                    // Flush after the fence, never before: the fence has to be
                    // in the stream it is meant to mark the end of, and a
                    // compositor waiting on an unflushed fence would hang.
                    self.gl.flush();
                    Some(FenceHandle(fence.0.cast()))
                }
                super::fence::GpuWait::Finish => {
                    self.gl.finish();
                    None
                }
            }
        }
    }

    /// Rebuild the widget in place, keeping the renderer and its targets.
    ///
    /// A build that fails leaves the view with no runtime rather than the old
    /// one: the reload was asked for because the wasm on disk changed, and
    /// carrying on with the previous build would show a widget that no longer
    /// exists.
    fn reload(&mut self, seed: super::ViewSeed, rebind: super::Rebind) -> Result<ViewInfo, String> {
        match seed.build_runtime() {
            Ok(runtime) => {
                self.core.install_runtime(runtime, rebind.led_rx);
                self.core
                    .rebind_credentials(*rebind.credentials, *rebind.secrets);
                Ok(self.core.info())
            }
            Err(e) => {
                self.core.install_runtime(None, None);
                Err(format!("{e:#}"))
            }
        }
    }

    fn destroy(mut self) {
        self.core.renderer_mut().drop_all();
        // The render target was never registered, so its texture is this
        // thread's to delete; the present textures belong to the painter.
        self.targets.destroy(&self.gl);
        for present in self.present {
            present.destroy_container_objects(&self.gl);
        }
    }
}

#[cfg(test)]
mod reload_ack_tests {
    use super::{FromWorker, Worker};
    use crate::view::ViewInfo;

    /// A worker whose thread is the test itself: the sender stands in for it,
    /// so a reload's answer can be posted, withheld, or refused.
    fn worker() -> (Worker, std::sync::mpsc::Sender<FromWorker>) {
        let (tx, _unused) = std::sync::mpsc::channel();
        let (worker_tx, rx) = std::sync::mpsc::channel();
        let worker = Worker {
            tx,
            rx,
            join: None,
            info: ViewInfo::default(),
            report: crate::view::ViewReport::default(),
            label: "test".to_owned(),
            showing: 0,
            lost: false,
        };
        (worker, worker_tx)
    }

    #[test]
    fn a_rebuilt_widget_replaces_the_info_the_view_reports() {
        let (mut worker, answers) = worker();
        let rebuilt = ViewInfo {
            live: true,
            sdk_version: Some((1, 2, 3)),
            exports_on_touch: true,
        };
        answers
            .send(FromWorker::Reloaded(Ok(rebuilt)))
            .expect("BUG: the worker holds the receiver");

        worker
            .take_reload_ack()
            .expect("a rebuilt widget must report success");

        assert_eq!(
            worker.info().sdk_version,
            Some((1, 2, 3)),
            "the reload's info must replace what was read at construction",
        );
    }

    #[test]
    fn a_failed_rebuild_reports_the_workers_own_reason() {
        let (mut worker, answers) = worker();
        answers
            .send(FromWorker::Reloaded(Err("no such export".to_owned())))
            .expect("BUG: the worker holds the receiver");

        let reason = worker
            .take_reload_ack()
            .expect_err("a failed rebuild must not read as a swap");

        assert_eq!(reason, "no such export");
        assert!(
            !worker.info().live,
            "a view that failed to rebuild is not live",
        );
    }

    /// A silent thread is a failed reload, not a successful one: the caller
    /// reports the swap on `Ok`.
    #[test]
    fn a_thread_that_never_answers_fails_the_reload() {
        let (mut worker, answers) = worker();
        drop(answers);

        worker
            .take_reload_ack()
            .expect_err("a dead channel must fail the reload");
    }
}

#[cfg(test)]
mod reap_tests {
    use super::Retired;

    fn retired(join: std::thread::JoinHandle<()>) -> Retired {
        Retired {
            join: Some(join),
            label: "test".to_owned(),
            closed_at: std::time::Instant::now(),
            warned: false,
        }
    }

    #[test]
    fn a_worker_still_winding_down_is_handed_back_not_waited_for() {
        let (hold_tx, hold_rx) = std::sync::mpsc::channel::<()>();
        let join = std::thread::spawn(move || {
            let _ = hold_rx.recv();
        });

        let still_going = retired(join)
            .reap()
            .expect("a blocked worker must be handed back");

        // Release the thread, then reaping converges without ever blocking.
        hold_tx
            .send(())
            .expect("BUG: the worker holds the receiver");
        let mut current = still_going;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match current.reap() {
                None => break,
                Some(again) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "a released worker must reap promptly"
                    );
                    current = again;
                    std::thread::yield_now();
                }
            }
        }
    }

    #[test]
    fn a_panicked_worker_reaps_without_propagating() {
        let join = std::thread::spawn(|| panic!("worker died"));
        while !join.is_finished() {
            std::thread::yield_now();
        }

        assert!(
            retired(join).reap().is_none(),
            "a finished worker is collected on the first poll"
        );
    }
}
