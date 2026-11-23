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

//! A widget as a struct, with parts of it as SDK-owned boxes.
//!
//! `#[widget]` on the `impl Widget for …` block generates the exports the
//! host calls, each of which runs one method of the one instance the SDK owns.
//! An SDK call that takes a callback first takes what the callback runs on, a [`Receiver`]:
//! `Timer::start(self, period, Self::tick)` runs `tick` as a method of the widget,
//! and `Timer::start(&self.session, period, Session::tick)` as a method of a [`Part`]'s value,
//! so a sub-object owns its own callbacks without reaching back through the widget.
//!
//! The SDK calls into the widget only from the top of a host-called export,
//! never from inside an SDK call the widget made.
//! Work a method or callback queues, such as a poll's next request,
//! runs once that method or callback has returned, before the export ends.
//! `docs/devel/wasm-widgets/widget-struct.md` walks through the order.
//!
//! A value sits in a `RefCell`, borrowed for one callback.
//! A trap inside a method (out of fuel, a panic) leaves that borrow standing,
//! which is harmless: the host never calls into a trapped instance again.
//!
//! ```ignore
//! use bmc_wasm_sdk::{Building, PollConfig, Poll, Widget, widget};
//!
//! struct RandomFact {
//!     fact: Option<String>,
//!     _poll: Poll,
//! }
//!
//! #[widget]
//! impl Widget for RandomFact {
//!     fn init(me: &Building<Self>) -> Self {
//!         let poll = Poll::register(me, Self::build_request, Self::on_reply, PollConfig::default());
//!         Self { fact: None, _poll: poll }
//!     }
//!
//!     fn render(&mut self, _delta_ms: u32) {
//!         // …
//!     }
//! }
//! ```

use core::cell::{Cell, OnceCell, RefCell};
use core::marker::PhantomData;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::thread::LocalKey;

/// The hooks the host calls.
/// `#[widget]` exports `init`, `render` and `unload`, and each other hook the impl writes.
pub trait Widget: Sized + 'static {
    /// Runs once the host has staged params, system state and credentials.
    ///
    /// Register the widget's own methods as callbacks through `me`.
    fn init(me: &Building<Self>) -> Self;

    /// Draws a frame; submit the tree with `render_ui`.
    fn render(&mut self, delta_ms: u32);

    /// Runs before the widget's first frame, and again each time it returns on-scene.
    fn on_wake(&mut self) {}

    /// Runs when the widget scrolls off-scene.
    fn on_sleep(&mut self) {}

    /// The host delivers touches only to a widget that writes this hook.
    fn on_touch(&mut self) {}

    /// Runs after each deck-wide system snapshot that follows the one staged for `init`.
    fn on_system_update(&mut self) {}

    /// Runs after each params snapshot that follows the one staged for `init`.
    fn on_params_update(&mut self) {}

    /// Runs after each credential delivery, secret rotations included.
    fn on_credentials_update(&mut self) {}

    /// Runs when the Deck's SSID or IP changes, in any lifecycle state.
    fn on_network_update(&mut self) {}

    /// The last call the widget gets; the instance is dropped afterwards.
    fn unload(self) {}
}

/// The widget whose `impl Widget` carries `#[widget]`; the SDK holds its one instance.
///
/// The attribute implements it with a root slot of the widget's own type,
/// so passing `self` binds a callback to that type at compile time.
#[diagnostic::on_unimplemented(
    message = "only the `#[widget]` type passes `self` as a receiver, and `{Self}` is not it",
    note = "a part's value passes the `PartHandle` it kept from `me.handle()`"
)]
pub trait Root: Widget {
    #[doc(hidden)]
    fn __root() -> &'static LocalKey<RootSlot<Self>>;

    /// The widget as a receiver that borrows nothing, to pass to code that runs
    /// on a field of the widget: `self.field.run(self.me())`.
    fn me(&self) -> Me<Self> {
        Me(PhantomData)
    }
}

/// What a callback runs on: the SDK delivers it as a method of this receiver's value.
///
/// Every SDK call that takes a method callback takes its receiver first, by reference:
///
/// - `self` in a method of the widget, or [`Me`] in code that runs on a field of it;
///   both name the instance the SDK holds, whatever value of the widget's type is passed;
/// - `me`, the [`Building`] that `init` and [`Part::new_cyclic`] lend;
/// - `&part`, a [`Part`] that is held;
/// - a [`PartHandle`] the part kept, which may outlive it.
///
/// The widget, a [`Building`] and a [`Part`] are there by construction,
/// so a registration through them fails only when the host refuses it, with [`RegisterError`].
/// Through a [`PartHandle`] it returns [`HandleError`] instead,
/// whose [`PartError`] says the handle is not live, so the host was never asked;
/// a poll registration or a `Notify` through one fails only with that `PartError`. See [`Outcome`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot receive callbacks for `{T}`'s methods",
    label = "not a receiver of `{T}`",
    note = "pass `self` in a method of the widget, `self.me()` to code on a field of it, \
            `me` while building, `&part`, or a `PartHandle` the part kept from `me.handle()`"
)]
pub trait Receiver<T>: sealed::Sealed {
    /// What a registration that returns `O` through the widget, a [`Building`] or a [`Part`]
    /// returns through this one:
    /// `O` itself, or [`Outcome::OrNotLive`] through a [`PartHandle`].
    type Out<O: Outcome>;

    /// Register with `register` on the target, if it is still there.
    #[doc(hidden)]
    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<T>) -> O) -> Self::Out<O>;
}

pub(crate) mod sealed {
    pub trait Sealed {}

    pub trait SealedOutcome {}

    /// A registration the host cannot refuse,
    /// whose only failure through a `PartHandle` is a part that is not live.
    #[diagnostic::on_unimplemented(
        message = "`{Self}` is not what an SDK registration returns",
        note = "a registration returns a `Result` with a `RegisterError`, a `Poll` or a `Notify`"
    )]
    pub trait Unrefusable {}
}

/// A registration that did not happen: the host refused it on the spot.
///
/// The host logs why when a limit refuses it,
/// but refuses input it cannot use, such as an empty list, without a log.
/// One past a limit may succeed later;
/// a fetch the host took but will never send ends in its callback,
/// with [`FetchOutcome::Refused`](crate::FetchOutcome::Refused).
#[derive(Clone, PartialEq, Eq)]
pub struct RegisterError<E = ()> {
    handed_back: E,
}

impl<E> RegisterError<E> {
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) const fn new(handed_back: E) -> Self {
        Self { handed_back }
    }

    /// A refusal for a widget's own tests to hand the code they test,
    /// behind the SDK's `test-support` feature.
    #[cfg(feature = "test-support")]
    pub fn for_test(handed_back: E) -> Self {
        Self { handed_back }
    }

    /// What the refused form hands back, such as the body of a refused decode.
    pub fn into_inner(self) -> E {
        self.handed_back
    }
}

impl<E: core::fmt::Debug> core::fmt::Debug for RegisterError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if size_of::<E>() == 0 {
            f.write_str("RegisterError")
        } else {
            f.debug_tuple("RegisterError")
                .field(&self.handed_back)
                .finish()
        }
    }
}

/// Why a [`PartHandle`] could not register at all: the host was never asked.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartError {
    /// The handle's part is dropped or still being built.
    /// Inside the build, register through the [`Building`] instead.
    NotLive,
}

/// Why a registration through a [`PartHandle`] did not happen:
/// it either never reached the host or the host refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleError<E = ()> {
    /// The handle could not register, so the host was never asked.
    Part(PartError),
    /// The host refused, as it would through any other receiver.
    Register(RegisterError<E>),
}

impl<E> From<PartError> for HandleError<E> {
    fn from(error: PartError) -> Self {
        Self::Part(error)
    }
}

impl<E> From<RegisterError<E>> for HandleError<E> {
    fn from(error: RegisterError<E>) -> Self {
        Self::Register(error)
    }
}

/// What a registration returns, and what it turns into through a [`PartHandle`]:
///
/// | Through any other receiver | Through a `PartHandle` |
/// |---|---|
/// | `Result<X, RegisterError<E>>` | `Result<X, HandleError<E>>` |
/// | a [`Poll`](crate::Poll) or a [`Notify`](crate::Notify), which the host cannot refuse | `Result<_, PartError>` |
pub trait Outcome: sealed::SealedOutcome + Sized {
    /// This outcome through a [`PartHandle`].
    type OrNotLive;

    #[doc(hidden)]
    fn __live(self) -> Self::OrNotLive;

    #[doc(hidden)]
    fn __not_live() -> Self::OrNotLive;
}

impl<X, E> sealed::SealedOutcome for Result<X, RegisterError<E>> {}

impl<X, E> Outcome for Result<X, RegisterError<E>> {
    type OrNotLive = Result<X, HandleError<E>>;

    fn __live(self) -> Self::OrNotLive {
        self.map_err(HandleError::Register)
    }

    fn __not_live() -> Self::OrNotLive {
        Err(HandleError::Part(PartError::NotLive))
    }
}

impl<H: sealed::Unrefusable> sealed::SealedOutcome for H {}

impl<H: sealed::Unrefusable> Outcome for H {
    type OrNotLive = Result<H, PartError>;

    fn __live(self) -> Self::OrNotLive {
        Ok(self)
    }

    fn __not_live() -> Self::OrNotLive {
        Err(PartError::NotLive)
    }
}

impl<W: Root> sealed::Sealed for W {}

impl<W: Root> Receiver<W> for W {
    type Out<O: Outcome> = O;

    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<W>) -> O) -> O {
        register(root())
    }
}

/// The widget as a [`Receiver`] that borrows nothing, from [`Root::me`].
pub struct Me<W>(PhantomData<fn() -> W>);

impl<W> Clone for Me<W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for Me<W> {}

impl<W> core::fmt::Debug for Me<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Me").finish_non_exhaustive()
    }
}

impl<W: Root> sealed::Sealed for Me<W> {}

impl<W: Root> Receiver<W> for Me<W> {
    type Out<O: Outcome> = O;

    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<W>) -> O) -> O {
        register(root())
    }
}

/// A widget or part while it is built, lent to register its own methods as callbacks.
///
/// A callback registered through it delivers once the value is built.
/// Keep a [`handle`](Self::handle) in the value to register more later.
pub struct Building<T>(Target<T>);

impl<T> Building<T> {
    /// A handle to the value being built, to keep and register through later.
    ///
    /// It counts as live only once the build has returned,
    /// so registering through it from inside the build fails with [`PartError::NotLive`],
    /// inside a [`HandleError`] where the host may refuse the registration.
    #[must_use]
    pub fn handle(&self) -> PartHandle<T> {
        PartHandle(self.0.clone())
    }
}

impl<T> core::fmt::Debug for Building<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Building").finish_non_exhaustive()
    }
}

impl<T> sealed::Sealed for Building<T> {}

impl<T> Receiver<T> for Building<T> {
    type Out<O: Outcome> = O;

    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<T>) -> O) -> O {
        register(self.0.clone())
    }
}

type Deferred = VecDeque<Box<dyn FnOnce()>>;

thread_local! {
    /// The running SDK export's deferred work; `None` outside one, where nothing would drain it.
    static DEFERRED: RefCell<Option<Deferred>> = const { RefCell::new(None) };
}

/// An SDK-owned box for a part of the widget whose methods are callbacks.
///
/// It owns its value like a `Box`: lend the value out with [`with`](Self::with)
/// or [`with_mut`](Self::with_mut), and drop the part to drop the value
/// and with it the handles the value holds.
/// Pass `&part` as the receiver of an SDK call (`Timer::start(&part, period, T::tick)`),
/// and the callback runs as that method on the value for as long as the part lives.
///
/// # Panics
///
/// A callback runs with the value borrowed mutably,
/// so reaching the part from its own callback panics rather than alias the value.
/// The callback reaches its part only through ownership shared outside the widget:
/// a `thread_local` holding the part, or an `Rc` around it that the value can reach.
pub struct Part<T: 'static>(Rc<RefCell<T>>);

impl<T> core::fmt::Debug for Part<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Part").finish_non_exhaustive()
    }
}

impl<T: 'static> Part<T> {
    /// A part holding `value`.
    /// A value that registers its own methods is built with [`new_cyclic`](Self::new_cyclic) instead.
    pub fn new(value: T) -> Self {
        Self(Rc::new(RefCell::new(value)))
    }

    /// Build the value with the part it is going into lent as `me`,
    /// to register the value's own methods as callbacks
    /// and to keep a [`handle`](Building::handle) for registering more later.
    pub fn new_cyclic(build: impl FnOnce(&Building<T>) -> T) -> Self {
        Self(Rc::new_cyclic(|cell| {
            RefCell::new(build(&Building(Target(cell.clone()))))
        }))
    }

    pub(crate) fn target(&self) -> Target<T> {
        Target(Rc::downgrade(&self.0))
    }

    /// Call `f` with the value.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self
            .0
            .try_borrow()
            .expect("BUG: a Part reached from its own callback"))
    }

    /// Call `f` with the value borrowed mutably; see [`with`](Self::with).
    pub fn with_mut<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self
            .0
            .try_borrow_mut()
            .expect("BUG: a Part reached from its own callback"))
    }
}

/// A handle to a [`Part`] that the part's value keeps, to register its own methods later.
///
/// A value cannot reach its own part from its callbacks,
/// but it can keep this from [`Building::handle`] and pass it as the receiver.
/// The handle never keeps the part alive: once the part is dropped,
/// registering through it fails with [`PartError::NotLive`],
/// inside a [`HandleError`] where the host may refuse the registration,
/// and a callback registered through it before delivers nothing.
/// A callback that drops its own part still registers until it returns,
/// and that registration delivers nothing.
pub struct PartHandle<T>(Target<T>);

impl<T> sealed::Sealed for PartHandle<T> {}

impl<T> Receiver<T> for PartHandle<T> {
    type Out<O: Outcome> = O::OrNotLive;

    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<T>) -> O) -> O::OrNotLive {
        if self.0.is_live() {
            register(self.0.clone()).__live()
        } else {
            O::__not_live()
        }
    }
}

impl<T> Clone for PartHandle<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> core::fmt::Debug for PartHandle<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PartHandle").finish_non_exhaustive()
    }
}

impl<T: 'static> sealed::Sealed for Part<T> {}

impl<T: 'static> Receiver<T> for Part<T> {
    type Out<O: Outcome> = O;

    fn __register<O: Outcome>(&self, register: impl FnOnce(Target<T>) -> O) -> O {
        register(self.target())
    }
}

/// Where a registered callback delivers: a part or the root, held weakly so
/// the registration never keeps the value alive.
#[doc(hidden)]
pub struct Target<T>(Weak<RefCell<T>>);

impl<T> Clone for Target<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> core::fmt::Debug for Target<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Target").finish_non_exhaustive()
    }
}

/// The root instance as a delivery target.
pub(crate) fn root<W: Root>() -> Target<W> {
    W::__root().with(RootSlot::target)
}

impl<T> Target<T> {
    /// Whether the value is built and not dropped.
    pub(crate) fn is_live(&self) -> bool {
        self.0.strong_count() > 0
    }
}

impl<T: 'static> Target<T> {
    /// Run `f` on the value as one callback, or do nothing once the part or
    /// root is gone.
    ///
    /// Only from the top of a host-called export or from [`drain`]: nothing
    /// else may hold a reference into the value then.
    pub(crate) fn deliver<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        assert!(
            DEFERRED.with_borrow(Option::is_some),
            "BUG: a delivery outside an SDK export; an `__on_*` export lacks its widget::export"
        );
        // Held for the callback: a part dropped from its own callback goes once it returns.
        let cell = self.0.upgrade()?;
        Some(f(&mut cell.try_borrow_mut().expect(
            "BUG: a delivery to a value something still has borrowed: \
             a method called an SDK export",
        )))
    }
}

/// The thread-local that holds a `#[widget]` type's instance.
#[doc(hidden)]
pub struct RootSlot<W> {
    /// Set as `init` starts, so the callbacks it registers reach the instance.
    target: OnceCell<Weak<RefCell<W>>>,
    /// The instance's one strong reference, from `init` to `unload`.
    instance: Cell<Option<Rc<RefCell<W>>>>,
}

impl<W> RootSlot<W> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            target: OnceCell::new(),
            instance: Cell::new(None),
        }
    }

    fn target(&self) -> Target<W> {
        Target(
            self.target
                .get()
                .cloned()
                .expect("BUG: the widget reached before the SDK installed it"),
        )
    }
}

impl<W> core::fmt::Debug for RootSlot<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RootSlot").finish_non_exhaustive()
    }
}

impl<W> Default for RootSlot<W> {
    fn default() -> Self {
        Self::new()
    }
}

/// Stand where an export's body stands, for a test outside one.
#[cfg(test)]
pub(crate) fn enter() {
    DEFERRED.with_borrow_mut(|deferred| {
        deferred.get_or_insert_default();
    });
}

/// Stand where the host stands between exports, once the deferred work ran.
#[cfg(test)]
pub(crate) fn leave() {
    drain();
    DEFERRED.set(None);
}

/// Queue work that calls back into the widget until the running method has returned.
#[cfg_attr(
    not(any(target_arch = "wasm32", test)),
    expect(
        dead_code,
        reason = "the poll glue and Notify that use it exist on wasm32 and in tests"
    )
)]
pub(crate) fn defer(op: impl FnOnce() + 'static) {
    DEFERRED.with_borrow_mut(|deferred| {
        deferred
            .as_mut()
            .expect(
                "BUG: a Notify or a method poll driven outside an SDK export; \
                 use a #[widget] hook or an SDK callback",
            )
            .push_back(Box::new(op));
    });
}

/// Run the deferred work, and what it defers in turn, in the order it was queued;
/// outside an export there is none.
pub(crate) fn drain() {
    while let Some(op) = DEFERRED.with_borrow_mut(|deferred| deferred.as_mut()?.pop_front()) {
        op();
    }
}

/// The `init` export: create the instance.
#[doc(hidden)]
pub fn install<W: Root>() {
    export(|| {
        let instance = Rc::new_cyclic(|cell| {
            let first = W::__root().with(|slot| slot.target.set(cell.clone()).is_ok());
            assert!(first, "BUG: the host calls init once");
            RefCell::new(W::init(&Building(Target(cell.clone()))))
        });
        W::__root().with(|slot| slot.instance.set(Some(instance)));
    });
}

/// A hook export: run `hook` on the instance, then the work it deferred.
#[doc(hidden)]
pub fn hook<W: Root>(hook: impl FnOnce(&mut W)) {
    export(|| {
        root::<W>().deliver(hook);
    });
}

/// The `unload` export: take the instance out and give it its last call.
#[doc(hidden)]
pub fn uninstall<W: Root>() {
    export(|| {
        let Some(instance) = W::__root().with(|slot| slot.instance.take()) else {
            return;
        };
        let cell = Rc::into_inner(instance).expect(
            "BUG: something besides the slot holds the instance between exports, \
             so it would never get its unload",
        );
        cell.into_inner().unload();
    });
}

/// Run `body` as a host-called export: it may deliver to the widget,
/// and the work it defers runs once it returns.
///
/// One run inside another, as a test acting for the host does, ends where it began.
#[doc(hidden)]
pub fn export(body: impl FnOnce()) {
    let outer = DEFERRED.replace(Some(VecDeque::new()));
    body();
    drain();
    DEFERRED.set(outer);
}

/// Stand in for the host around a call of an export,
/// which a `#[wasm_test]` body cannot do by itself as it runs inside one.
#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) fn as_host(call: impl FnOnce()) {
    leave();
    call();
    assert!(
        DEFERRED.with_borrow(Option::is_none),
        "an export must end with no export running"
    );
    enter();
}

#[cfg(test)]
mod tests;
