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

//! The poll registry behind the SDK's [`Handle`], and the forms that register one.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;

use crate::net::FetchResponse;
use crate::widget::{self, Receiver, Target};

use super::backend::{HostBackend, now_secs};
use super::{
    Bound, Build, BuildTiming, Config, FetchSpec, Free, Handle, Kick, Poll, Registry, ReplyAction,
    TargetGone, Verdict,
};

/// Receives each reply with the borrowed handle of the poll it belongs to.
pub type OnReply = fn(&Handle, &FetchResponse);

/// A poll's `build` as a method of the value it is delivered to.
pub type BuildMethod<T> = fn(&mut T, &Poll) -> Option<FetchSpec>;

/// A poll's reply as a method of the value it is delivered to, which says how the reply counts.
pub type ReplyMethod<T> = fn(&mut T, &Poll, &FetchResponse) -> Verdict;

thread_local! {
    pub(super) static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Perform `kick` now, or once the running method has returned
/// when its `build` reaches into the widget.
fn schedule(kick: Option<Kick>) {
    let Some(kick) = kick else {
        return;
    };
    match kick.build_timing {
        BuildTiming::AtExportEnd => widget::defer(move || perform(kick)),
        BuildTiming::Now => perform(kick),
    }
}

/// Run the poll's `build` outside the registry borrow and send what it made.
fn perform(kick: Kick) {
    let Some(build) = REGISTRY.with_borrow_mut(|registry| registry.prepare(&kick)) else {
        return;
    };
    let Ok(spec) = build(kick.index) else {
        REGISTRY.with_borrow_mut(|registry| {
            registry.finish(&kick, None, &mut HostBackend);
            registry.release(kick.index, &mut HostBackend);
        });
        return;
    };
    REGISTRY.with_borrow_mut(|registry| registry.finish(&kick, spec.as_ref(), &mut HostBackend));
}

pub(super) fn trampoline(response: &FetchResponse) {
    let action = REGISTRY.with_borrow_mut(|registry| registry.begin_reply(response.request_id));
    match action {
        ReplyAction::Ignore | ReplyAction::Stopped => {}
        ReplyAction::Reissued(kick) => schedule(Some(kick)),
        ReplyAction::Deliver(index, on_reply) => {
            let Ok(verdict) = on_reply(index, response) else {
                REGISTRY.with_borrow_mut(|registry| registry.release(index, &mut HostBackend));
                return;
            };
            let now = now_secs();
            let delay = REGISTRY.with_borrow_mut(|registry| {
                registry.bank_reply(index, response.ok(), verdict, now)
            });
            // Banked first, so a `Notify` the handler sent sees this reply's freshness;
            // what the handler asked for (`enable_after`, `invalidate`) goes out first,
            // and the interval's kick then leaves it be.
            widget::drain();
            schedule(REGISTRY.with_borrow_mut(|registry| registry.kick_after(index, delay)));
        }
    }
}

/// Register a poll and send its first request, unless `config.enabled` is false.
///
/// An enabled poll's `build` runs at once; a disabled one's waits until it is enabled.
/// Returns the only handle to the poll; dropping it ends the poll.
///
/// # Panics
///
/// From inside any poll's `build`,
/// where the new poll would build before that build's request goes out.
#[must_use = "dropping the returned Handle ends the poll"]
pub fn register(build: Build, on_reply: OnReply, config: Config) -> Handle {
    register_with(
        move |index| Ok(build(&Handle::borrowed(index))),
        move |index, response| {
            on_reply(&Handle::borrowed(index), response);
            // A rejection reaches the registry through the handle's `retry`.
            Ok(Verdict::Accept)
        },
        BuildTiming::Now,
        config,
    )
}

fn register_with<K>(
    build: impl Fn(usize) -> Result<Option<FetchSpec>, TargetGone> + 'static,
    on_reply: impl Fn(usize, &FetchResponse) -> Result<Verdict, TargetGone> + 'static,
    build_timing: BuildTiming,
    config: Config,
) -> Handle<K> {
    let index = REGISTRY
        .with_borrow_mut(|registry| registry.register(build, on_reply, build_timing, config));
    schedule(REGISTRY.with_borrow_mut(|registry| registry.start(index)));
    Handle(index, PhantomData)
}

fn register_on_target<T: 'static>(
    target: Target<T>,
    build: BuildMethod<T>,
    on_reply: ReplyMethod<T>,
    config: Config,
) -> Poll {
    let reply_target = target.clone();
    // A method that dropped its own part ends the poll too.
    register_with(
        move |index| {
            target
                .deliver(|value| build(value, &Poll::borrowed(index)))
                .filter(|_| target.is_live())
                .ok_or(TargetGone)
        },
        move |index, response| {
            reply_target
                .deliver(|value| on_reply(value, &Poll::borrowed(index), response))
                .filter(|_| reply_target.is_live())
                .ok_or(TargetGone)
        },
        BuildTiming::AtExportEnd,
        config,
    )
}

impl<K> Drop for Handle<K> {
    fn drop(&mut self) {
        REGISTRY.with_borrow_mut(|registry| registry.release(self.0, &mut HostBackend));
    }
}

impl widget::sealed::Unrefusable for Poll {}

impl Handle<Bound> {
    /// Register a poll whose `build` and `on_reply` run as methods of `to`.
    ///
    /// Otherwise as [`register`], except that `build` runs once the method
    /// or callback that asked for the request has returned, and of the requests
    /// asked for by then the last one wins, though neither `enable_after`
    /// nor `set_enabled(true)` replaces one on an enabled poll.
    /// Drive the poll from `#[widget]` hooks or SDK callbacks only.
    /// `on_reply` rejects a reply by returning [`Verdict::Retry`] or [`Verdict::RetryAfter`].
    ///
    /// Through a [`PartHandle`](crate::PartHandle) it returns a `Result`,
    /// as [`Outcome`](crate::Outcome) says.
    /// Once the part is dropped, the poll ends the next time it would build or deliver a reply,
    /// as if stopped.
    ///
    /// # Panics
    ///
    /// As [`invalidate`](Self::invalidate).
    #[must_use = "dropping the returned Poll ends the poll"]
    pub fn register<T: 'static, R: Receiver<T>>(
        to: &R,
        build: BuildMethod<T>,
        on_reply: ReplyMethod<T>,
        config: Config,
    ) -> R::Out<Poll> {
        to.__register(|target| register_on_target(target, build, on_reply, config))
    }
}

impl<K> Handle<K> {
    /// The handle a callback borrows; dropping it would end the poll.
    pub(super) fn borrowed(index: usize) -> ManuallyDrop<Self> {
        ManuallyDrop::new(Self(index, PhantomData))
    }

    /// End the poll: cancel its live fetch, and drop a reply already on its way.
    pub fn stop(self) {
        drop(self);
    }

    /// Enable the poll and ask for a request, or disable it and cancel its live one.
    ///
    /// # Panics
    ///
    /// When enabling, as [`invalidate`](Self::invalidate).
    pub fn set_enabled(&self, enabled: bool) {
        schedule(
            REGISTRY.with_borrow_mut(|registry| {
                registry.set_enabled(self.0, enabled, &mut HostBackend)
            }),
        );
    }

    /// Enable with the next fetch after `delay_ms` instead of the debounce.
    ///
    /// On an enabled poll, a fetch already asked for keeps its timing;
    /// [`invalidate`](Self::invalidate) is what replaces it.
    /// On a disabled poll whose request the host would not cancel,
    /// the delay counts from that request's reply.
    ///
    /// # Panics
    ///
    /// As [`invalidate`](Self::invalidate).
    pub fn enable_after(&self, delay_ms: u32) {
        schedule(REGISTRY.with_borrow_mut(|registry| registry.set_enabled_after(self.0, delay_ms)));
    }

    /// Retarget the poll cadence and `is_stale` threshold; pair with `invalidate` to apply now.
    pub fn set_interval(&self, interval_ms: u32) {
        REGISTRY.with_borrow_mut(|registry| registry.set_interval(self.0, interval_ms));
    }

    /// Ask for a fresh request in place of the live one.
    ///
    /// # Panics
    ///
    /// From inside any poll's `build`,
    /// where it would build again before that build's request goes out.
    /// For a poll with method callbacks, also outside an SDK export, such as a hand-written one,
    /// where nothing would run the `build`.
    pub fn invalidate(&self) {
        schedule(
            REGISTRY.with_borrow_mut(|registry| registry.invalidate(self.0, &mut HostBackend)),
        );
    }

    /// Count the last ok reply as failed after all (e.g. a late decode failure),
    /// even if failed replies banked since; with none left to undo, count a plain failure.
    ///
    /// A reply handler rejects the reply it is handling with its [`Verdict`],
    /// or `retry` in the free form, instead:
    /// that reply banks only once the handler returns, so this would undo the one before it.
    pub fn mark_stale(&self) {
        let now = now_secs();
        REGISTRY.with_borrow_mut(|registry| registry.mark_stale(self.0, now));
    }

    /// Seed the last-success anchor from a restored cache entry
    /// so staleness reads the true age across a reboot.
    pub fn restore_anchor(&self, unix_secs: i64) {
        REGISTRY.with_borrow_mut(|registry| registry.restore_anchor(self.0, unix_secs));
    }

    /// Forget all staleness history — for when the widget blanks its data.
    pub fn reset_staleness(&self) {
        REGISTRY.with_borrow_mut(|registry| registry.reset_staleness(self.0));
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        REGISTRY.with_borrow(|registry| registry.enabled(self.0))
    }

    /// Whether the poll is failing with stale data.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        let now = now_secs();
        REGISTRY.with_borrow(|registry| registry.is_stale(self.0, now))
    }

    /// Never loaded and failing past the stale grace.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        let now = now_secs();
        REGISTRY.with_borrow(|registry| registry.is_offline(self.0, now))
    }

    /// Age of the last successful reply, or `None` if it has never succeeded.
    #[must_use]
    pub fn last_success_age(&self) -> Option<std::time::Duration> {
        let now = now_secs();
        REGISTRY.with_borrow(|registry| registry.last_success_age(self.0, now))
    }

    /// The last successful reply's instant — the relative-time anchor.
    #[must_use]
    pub fn last_success_time(&self) -> Option<crate::host::SystemTime> {
        REGISTRY.with_borrow(|registry| {
            registry
                .last_success_secs(self.0)
                .map(|unix_secs| crate::host::SystemTime { unix_secs })
        })
    }
}

impl Handle<Free> {
    /// Treat the reply being delivered as unusable, for a body this handler cannot use despite the 2xx.
    ///
    /// The poll then reads as failing and keeps the anchor it had,
    /// and the next attempt waits `retry_ms` rather than the interval.
    /// Called outside the reply handler, it marks the next reply the poll counts.
    ///
    /// # Panics
    ///
    /// From inside any poll's `build`, which has no reply to retry.
    pub fn retry(&self) {
        REGISTRY.with_borrow_mut(|registry| registry.request_retry(self.0));
    }

    /// Like [`retry`](Self::retry), including counting as no refresh,
    /// but the next attempt waits `delay_ms` instead of `retry_ms`.
    ///
    /// # Panics
    ///
    /// From inside any poll's `build`, which has no reply to retry.
    pub fn retry_after(&self, delay_ms: u32) {
        REGISTRY.with_borrow_mut(|registry| registry.request_retry_after(self.0, delay_ms));
    }
}
