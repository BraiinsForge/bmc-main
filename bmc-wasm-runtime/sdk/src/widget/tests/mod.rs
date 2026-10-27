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

use core::cell::{Cell, RefCell};
use std::thread::LocalKey;

use super::{
    Building, Me, Part, PartHandle, Receiver, Root, RootSlot, Target, Widget, defer, drain, export,
    hook, install, leave, uninstall,
};
use crate::net::FetchResponse;
use crate::poll::{Config as PollConfig, FetchSpec, Handle as PollHandle};
use crate::wasm_test;

// Names the host's exports, which exist on wasm32 only.
#[cfg(target_arch = "wasm32")]
mod exports;

thread_local! {
    static ORDER: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static DROPPED: Cell<bool> = const { Cell::new(false) };
    static UNLOADED: Cell<bool> = const { Cell::new(false) };
    static STORED: RefCell<Option<Part<Counter>>> = const { RefCell::new(None) };
}

fn order() -> Vec<&'static str> {
    ORDER.with_borrow(Clone::clone)
}

fn note(step: &'static str) {
    ORDER.with_borrow_mut(|order| order.push(step));
}

struct Counter {
    n: usize,
}

impl Drop for Counter {
    fn drop(&mut self) {
        DROPPED.set(true);
    }
}

struct Sub {
    inner: Part<Counter>,
}

impl Sub {
    /// Reaches the widget from code that runs on a field of it.
    fn arm(&mut self, me: Me<Rooted>) -> Target<Rooted> {
        self.inner.with_mut(|counter| counter.n += 1);
        me.__target().expect("BUG: the widget is installed")
    }
}

/// The root of these tests: `init` and `unload` each defer a step and record that they ran.
struct Rooted {
    n: usize,
    sub: Sub,
}

impl Widget for Rooted {
    fn init(_: &Building<Self>) -> Self {
        defer(|| note("deferred from init"));
        note("init");
        Self {
            n: 0,
            sub: Sub {
                inner: Part::new(Counter { n: 0 }),
            },
        }
    }

    fn render(&mut self, _delta_ms: u32) {}

    fn unload(self) {
        defer(|| note("deferred from unload"));
        UNLOADED.set(true);
    }
}

impl Root for Rooted {
    fn __root() -> &'static LocalKey<RootSlot<Self>> {
        thread_local!(
            static ROOT: RootSlot<Rooted> = RootSlot::new();
        );
        &ROOT
    }
}

fn count() -> usize {
    let mut n = None;
    hook::<Rooted>(|rooted| n = Some(rooted.n));
    n.expect("BUG: the instance is installed")
}

#[wasm_test]
fn hooks_run_on_the_one_instance() {
    install::<Rooted>();
    hook::<Rooted>(|rooted| rooted.n += 1);
    hook::<Rooted>(|rooted| rooted.n += 1);

    assert_eq!(count(), 2);
}

#[wasm_test]
fn a_field_registers_on_the_widget_through_me_while_it_is_borrowed() {
    install::<Rooted>();
    let mut armed = None;
    hook::<Rooted>(|rooted| armed = Some(rooted.sub.arm(rooted.me())));
    armed
        .expect("BUG: set by the hook")
        .deliver(|rooted| rooted.n += 1);

    assert_eq!(
        count(),
        1,
        "the callback ran on the widget, not on the field"
    );
}

#[wasm_test]
#[should_panic(expected = "the host calls init once")]
fn a_second_init_is_a_bug() {
    install::<Rooted>();
    install::<Rooted>();
}

#[wasm_test]
fn work_deferred_from_init_runs_once_init_returned() {
    install::<Rooted>();

    assert_eq!(order(), ["init", "deferred from init"]);
}

#[wasm_test]
fn work_deferred_from_a_hook_runs_once_the_hook_returned() {
    install::<Rooted>();

    hook::<Rooted>(|_| defer(|| note("deferred from a hook")));

    assert_eq!(
        order(),
        ["init", "deferred from init", "deferred from a hook"],
        "the hook's export drains before it hands back to the host"
    );
}

#[wasm_test]
fn work_deferred_from_unload_runs_once_unload_returned() {
    install::<Rooted>();

    uninstall::<Rooted>();

    assert_eq!(
        order(),
        ["init", "deferred from init", "deferred from unload"],
        "the unload export drains before it hands back to the host"
    );
}

#[wasm_test]
fn unload_is_the_last_call_the_instance_gets() {
    install::<Rooted>();
    uninstall::<Rooted>();
    let mut ran = false;
    hook::<Rooted>(|_| ran = true);

    assert!(UNLOADED.get() && !ran, "unloaded, and nothing after it");
}

#[wasm_test]
#[should_panic(expected = "would never get its unload")]
fn an_instance_held_past_its_exports_is_a_bug() {
    install::<Rooted>();
    let _held = super::root::<Rooted>().0.upgrade();
    uninstall::<Rooted>();
}

/// A root whose `init` registers a method poll; its first `build` runs once `init` returned.
struct PollsFromInit {
    builds: usize,
    _poll: PollHandle,
}

impl PollsFromInit {
    fn build(&mut self, _poll: &PollHandle) -> Option<FetchSpec> {
        self.builds += 1;
        None
    }

    #[expect(
        clippy::unused_self,
        reason = "build sends nothing, so no reply arrives"
    )]
    fn on_reply(&mut self, _poll: &PollHandle, _response: &FetchResponse) {}
}

impl Widget for PollsFromInit {
    fn init(me: &Building<Self>) -> Self {
        Self {
            builds: 0,
            _poll: PollHandle::register(me, Self::build, Self::on_reply, PollConfig::default()),
        }
    }

    fn render(&mut self, _delta_ms: u32) {}
}

impl Root for PollsFromInit {
    fn __root() -> &'static LocalKey<RootSlot<Self>> {
        thread_local!(
            static ROOT: RootSlot<PollsFromInit> = RootSlot::new();
        );
        &ROOT
    }
}

#[wasm_test]
fn a_callback_registered_in_init_reaches_the_instance() {
    let mut builds = None;
    install::<PollsFromInit>();
    hook::<PollsFromInit>(|widget| builds = Some(widget.builds));
    uninstall::<PollsFromInit>();

    assert_eq!(
        builds,
        Some(1),
        "init's own poll built on the instance it was registered for"
    );
}

#[wasm_test]
fn a_callback_registered_through_the_widget_reaches_the_instance() {
    install::<PollsFromInit>();
    let mut kept = None;
    hook::<PollsFromInit>(|widget| {
        kept = Some(PollHandle::register(
            widget,
            PollsFromInit::build,
            PollsFromInit::on_reply,
            PollConfig::default(),
        ));
    });
    let mut builds = None;
    hook::<PollsFromInit>(|widget| builds = Some(widget.builds));
    uninstall::<PollsFromInit>();
    drop(kept);

    assert_eq!(
        builds,
        Some(2),
        "the poll registered through `self` built on the instance, after init's own"
    );
}

#[wasm_test]
fn a_target_taken_through_the_widget_reaches_the_instance() {
    install::<Rooted>();
    let mut target = None;
    hook::<Rooted>(|rooted| target = rooted.__target());

    target
        .expect("BUG: the widget is installed")
        .deliver(|rooted| rooted.n += 1);

    assert_eq!(
        count(),
        1,
        "every method form but the poll's binds `self` through its target"
    );
}

struct BuiltByHand;

impl Widget for BuiltByHand {
    fn init(_: &Building<Self>) -> Self {
        Self
    }

    fn render(&mut self, _delta_ms: u32) {}
}

impl Root for BuiltByHand {
    fn __root() -> &'static LocalKey<RootSlot<Self>> {
        thread_local!(
            static ROOT: RootSlot<BuiltByHand> = RootSlot::new();
        );
        &ROOT
    }
}

#[wasm_test]
#[should_panic(expected = "before the SDK installed it")]
fn a_widget_built_by_hand_cannot_register_its_methods() {
    let _ = PollHandle::register(
        &BuiltByHand,
        |_: &mut BuiltByHand, _: &PollHandle| None,
        |_: &mut BuiltByHand, _: &PollHandle, _: &FetchResponse| {},
        PollConfig::default(),
    );
}

#[wasm_test]
fn a_part_owned_by_the_instance_is_used_through_the_instance() {
    install::<Rooted>();
    hook::<Rooted>(|rooted| rooted.sub.inner.with_mut(|inner| inner.n += 2));
    let mut n = None;
    hook::<Rooted>(|rooted| n = Some(rooted.sub.inner.with(|inner| inner.n)));

    assert_eq!(n, Some(2));
}

#[wasm_test]
fn a_callback_on_a_part_runs_as_a_method_of_its_value() {
    let part = Part::new(Counter { n: 0 });
    let target = part.target();

    let seen = target.deliver(|counter| {
        counter.n += 1;
        counter.n
    });

    assert_eq!((seen, part.with(|counter| counter.n)), (Some(1), 1));
}

#[wasm_test]
#[should_panic(expected = "reached from its own callback")]
fn a_part_reached_from_its_own_callback_panics() {
    STORED.set(Some(Part::new(Counter { n: 0 })));
    let target = STORED.with_borrow(|stored| stored.as_ref().map(Part::target));

    target.expect("BUG: stored above").deliver(|_| {
        STORED.with_borrow(|stored| stored.as_ref().map(|part| part.with(|counter| counter.n)))
    });
}

#[wasm_test]
#[should_panic(expected = "reached from its own callback")]
fn a_part_reached_mutably_from_its_own_callback_panics() {
    STORED.set(Some(Part::new(Counter { n: 0 })));
    let target = STORED.with_borrow(|stored| stored.as_ref().map(Part::target));

    target.expect("BUG: stored above").deliver(|_| {
        STORED.with_borrow_mut(|stored| {
            stored
                .as_mut()
                .map(|part| part.with_mut(|counter| counter.n += 1))
        });
    });
}

#[wasm_test]
fn a_part_dropped_from_its_own_callback_goes_as_it_returns() {
    STORED.set(Some(Part::new(Counter { n: 0 })));
    let target = STORED.with_borrow(|stored| stored.as_ref().map(Part::target));

    target.expect("BUG: stored above").deliver(|counter| {
        STORED.take();
        counter.n += 1;
        note(if DROPPED.get() { "dropped" } else { "alive" });
    });
    note(if DROPPED.get() { "dropped" } else { "alive" });

    assert_eq!(order(), ["alive", "dropped"]);
}

#[wasm_test]
fn a_dropped_part_gets_no_delivery() {
    let part = Part::new(Counter { n: 0 });
    let target = part.target();
    drop(part);

    assert_eq!(target.deliver(|_| ()), None);
}

#[wasm_test]
fn a_part_dropped_from_its_own_callback_is_gone_before_deferred_work() {
    STORED.set(Some(Part::new(Counter { n: 0 })));
    let target = STORED
        .with_borrow(|stored| stored.as_ref().map(Part::target))
        .expect("BUG: stored above");
    target.deliver(|_| {
        defer(|| note(if DROPPED.get() { "dropped" } else { "alive" }));
        STORED.take();
    });
    drain();

    assert_eq!(order(), ["dropped"]);
}

#[wasm_test]
fn deferred_work_may_defer_more() {
    defer(|| {
        note("first");
        defer(|| note("second"));
    });
    drain();

    assert_eq!(order(), ["first", "second"]);
}

#[wasm_test]
#[should_panic(expected = "still has borrowed")]
fn a_delivery_to_a_part_still_borrowed_fails_closed() {
    let part = Part::new(Counter { n: 0 });
    core::mem::forget(part.0.borrow_mut());

    part.target().deliver(|_| ());
}

#[wasm_test]
#[should_panic(expected = "a delivery outside an SDK export")]
fn a_delivery_outside_an_export_is_a_bug() {
    let part = Part::new(Counter { n: 0 });
    leave();

    part.target().deliver(|_| ());
}

#[wasm_test]
fn an_export_inside_an_export_leaves_the_outer_one_running() {
    defer(|| note("outer"));

    export(|| defer(|| note("inner")));
    assert_eq!(
        order(),
        ["inner"],
        "the inner export runs only its own work"
    );

    drain();
    assert_eq!(order(), ["inner", "outer"]);
}

#[wasm_test]
#[should_panic(expected = "before the SDK installed it")]
fn a_hook_before_init_is_a_bug() {
    hook::<Rooted>(|_| ());
}

#[wasm_test]
fn a_registration_never_keeps_the_value_alive() {
    let part = Part::new(Counter { n: 0 });
    let target = part.target();
    drop(part);

    assert!(
        DROPPED.get() && target.0.strong_count() == 0,
        "the value went with its part"
    );
}

/// A value that keeps a handle to its own part, as a loader chaining fetches does.
struct Chained {
    me: PartHandle<Chained>,
    steps: usize,
}

impl Drop for Chained {
    fn drop(&mut self) {
        DROPPED.set(true);
    }
}

impl Chained {
    fn new(me: &Building<Chained>) -> Self {
        Self {
            me: me.handle(),
            steps: 0,
        }
    }

    /// Takes the next step through its own handle until it has taken `last`,
    /// the way a fetch reply registers the decode that follows it.
    fn step(&mut self, last: usize) {
        self.steps += 1;
        if self.steps < last {
            let me = self
                .me
                .__target()
                .expect("BUG: a part delivering to itself is live");
            defer(move || {
                me.deliver(|chained| chained.step(last));
            });
        }
    }
}

#[wasm_test]
fn a_callback_registers_its_next_one_on_its_own_part_through_its_handle() {
    let part = Part::new_cyclic(Chained::new);

    part.target().deliver(|chained| chained.step(3));
    drain();

    assert_eq!(
        part.with(|chained| chained.steps),
        3,
        "each step reached the part from inside the last"
    );
}

#[wasm_test]
fn a_handle_taken_while_building_registers_only_once_the_build_returned() {
    let mut refused_while_building = None;
    let part = Part::new_cyclic(|me| {
        refused_while_building = Some(me.handle().__target().is_none());
        Chained::new(me)
    });

    assert_eq!(
        refused_while_building,
        Some(true),
        "no value exists to deliver into yet"
    );
    assert!(part.with(|chained| chained.me.__target().is_some()));
}

#[wasm_test]
fn a_registration_while_building_delivers_once_the_value_is_built() {
    let mut registered = None;
    let part = Part::new_cyclic(|me| {
        let target = me.__target().expect("BUG: building always registers");
        assert_eq!(target.deliver(|_| ()), None, "nothing to deliver into yet");
        registered = Some(target);
        Chained::new(me)
    });

    registered
        .expect("BUG: set by the build")
        .deliver(|chained| chained.steps = 7);

    assert_eq!(part.with(|chained| chained.steps), 7);
}

#[wasm_test]
fn a_value_holding_its_own_handle_goes_with_its_part() {
    let part = Part::new_cyclic(Chained::new);
    let handle = part.with(|chained| chained.me.clone());
    drop(part);

    assert!(
        DROPPED.get(),
        "the handle it holds is no cycle keeping it alive"
    );
    assert!(
        handle.__target().is_none(),
        "registering through the handle is refused"
    );
}

#[wasm_test]
fn a_registration_from_a_callback_that_dropped_its_part_delivers_nothing() {
    let mut handle = None;
    STORED.set(Some(Part::new_cyclic(|me| {
        handle = Some(me.handle());
        Counter { n: 0 }
    })));
    let handle = handle.expect("BUG: set by the build");
    let mut registered = None;
    handle.__target().expect("BUG: stored above").deliver(|_| {
        STORED.take();
        registered = handle.__target();
    });

    assert_eq!(
        registered
            .expect("BUG: the value still runs its callback")
            .deliver(|_| ()),
        None
    );
}
