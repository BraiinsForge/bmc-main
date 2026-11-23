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

//! A callback a part fires into whoever made it, without naming that type.
//!
//! The widget hands a part a [`Notify`] bound to one of its own methods,
//! and the part sends through it when something happened:
//!
//! ```ignore
//! struct FactSource {
//!     on_fact: Notify<String>,
//! }
//!
//! impl FactSource {
//!     fn on_reply(&mut self, _poll: &Poll, response: &FetchResponse) -> Verdict {
//!         let Some(fact) = parse(response) else {
//!             return Verdict::Retry;
//!         };
//!         self.on_fact.send(fact);
//!         Verdict::Accept
//!     }
//! }
//!
//! // in the widget's `init`
//! let source = FactSource::new(Notify::new(me, Self::on_fact));
//! ```
//!
//! A send is delivered once the method or callback that sent it has returned, never inside it:
//! the widget owns the part, so a call straight into the widget would find the part still borrowed.

use std::rc::Rc;

use crate::widget;
use crate::widget::Receiver;

/// A notification's delivery as a method of the value it is delivered to.
pub type NotifyMethod<T, V> = fn(&mut T, V);

/// A method of another value, to run with a value of `V` each time the holder sends one.
pub struct Notify<V>(Rc<dyn Fn(V)>);

impl<V: 'static> Notify<V> {
    /// Bind `method` on `to`.
    ///
    /// Through a [`PartHandle`](crate::PartHandle) it returns a `Result`,
    /// as [`Outcome`](crate::Outcome) says.
    #[must_use]
    pub fn new<T: 'static, R: Receiver<T>>(to: &R, method: NotifyMethod<T, V>) -> R::Out<Self> {
        to.__register(|target| {
            Self(Rc::new(move |value| {
                target.deliver(|receiver| method(receiver, value));
            }))
        })
    }

    /// A `Notify` bound to nothing, whose sends run nothing:
    /// for a part built without a widget to tell, as in a test.
    #[must_use]
    pub fn unheard() -> Self {
        Self(Rc::new(|_| {}))
    }

    /// Run the bound method with `value` once the running method or callback has returned.
    ///
    /// Every send is delivered, in the order sent,
    /// even once this `Notify` is dropped or replaced: carry an id in `V` to tell senders apart.
    /// One sent to a value that is gone by then runs nothing.
    ///
    /// # Panics
    ///
    /// Outside an SDK export, such as a hand-written one, where nothing would deliver it.
    pub fn send(&self, value: V) {
        let deliver = Rc::clone(&self.0);
        widget::defer(move || deliver(value));
    }
}

impl<V> widget::sealed::Unrefusable for Notify<V> {}

impl<V> core::fmt::Debug for Notify<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Notify").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use super::Notify;
    use crate::wasm_test;
    use crate::widget;
    use crate::widget::{Part, PartHandle, Receiver};

    thread_local! {
        static SEEN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }

    fn seen() -> Vec<&'static str> {
        SEEN.with_borrow(Clone::clone)
    }

    fn note(step: &'static str) {
        SEEN.with_borrow_mut(|seen| seen.push(step));
    }

    struct Source {
        n: usize,
        on_change: Notify<usize>,
    }

    impl Source {
        fn bump(&mut self) {
            self.n += 1;
            self.on_change.send(self.n);
            note("sent");
        }
    }

    /// Owns its source, as a widget owns a part.
    #[derive(Default)]
    struct Owner {
        source: Option<Part<Source>>,
        seen: Vec<usize>,
        read: Option<usize>,
    }

    impl Owner {
        fn on_change(&mut self, n: usize) {
            note("delivered");
            self.seen.push(n);
            self.read = self
                .source
                .as_ref()
                .map(|source| source.with(|source| source.n));
        }
    }

    fn owner_with_source() -> Part<Owner> {
        let mut owner = Part::new(Owner::default());
        let source = Part::new(Source {
            n: 0,
            on_change: Notify::new(&owner, Owner::on_change),
        });
        owner.with_mut(|owner| owner.source = Some(source));
        owner
    }

    fn bump(owner: &Part<Owner>) {
        let source = owner.with(|owner| owner.source.as_ref().map(Part::target));
        source
            .expect("BUG: set by owner_with_source")
            .deliver(Source::bump);
    }

    #[wasm_test]
    fn a_send_is_delivered_once_the_sender_has_returned() {
        let owner = owner_with_source();

        bump(&owner);
        widget::drain();

        assert_eq!(seen(), ["sent", "delivered"]);
    }

    #[wasm_test]
    fn the_receiver_may_look_into_the_part_that_sent() {
        let owner = owner_with_source();

        bump(&owner);
        widget::drain();

        assert_eq!(
            owner.with(|owner| owner.read),
            Some(1),
            "the delivery reads the source the send came from"
        );
    }

    #[wasm_test]
    fn every_send_is_delivered_in_order() {
        let owner = owner_with_source();

        bump(&owner);
        bump(&owner);
        bump(&owner);
        widget::drain();

        assert_eq!(owner.with(|owner| owner.seen.clone()), [1, 2, 3]);
    }

    #[wasm_test]
    fn a_send_whose_receiver_is_dropped_before_delivery_runs_nothing() {
        let owner = Part::new(Owner::default());
        let notify = Notify::new(&owner, Owner::on_change);

        notify.send(1);
        drop(owner);
        widget::drain();

        assert!(seen().is_empty());
    }

    fn owner_and_handle() -> (Part<Owner>, PartHandle<Owner>) {
        let mut handle = None;
        let owner = Part::new_cyclic(|me| {
            handle = Some(me.handle());
            Owner::default()
        });
        (owner, handle.expect("BUG: set by the build"))
    }

    #[wasm_test]
    fn a_notify_through_a_dropped_part_handle_finds_it_dropped() {
        let (owner, handle) = owner_and_handle();
        drop(owner);

        assert!(matches!(
            Notify::new(&handle, Owner::on_change),
            Err(widget::PartError::NotLive)
        ));
    }

    fn notify_and_a_refusable_registration(
        handle: &PartHandle<Owner>,
    ) -> Result<Notify<usize>, widget::HandleError> {
        let notify = Notify::new(handle, Owner::on_change)?;
        handle.__register(|_| Ok::<_, widget::RegisterError>(()))?;
        Ok(notify)
    }

    #[wasm_test]
    fn a_dropped_notify_joins_the_error_of_registrations_the_host_may_refuse() {
        let (owner, handle) = owner_and_handle();
        drop(owner);

        assert!(
            matches!(
                notify_and_a_refusable_registration(&handle),
                Err(widget::HandleError::Part(widget::PartError::NotLive))
            ),
            "the `Notify`'s `PartError` leaves through the `HandleError` the other registration returns"
        );
    }

    #[wasm_test]
    fn a_notify_through_a_live_part_handle_is_delivered() {
        let (owner, handle) = owner_and_handle();
        let notify =
            Notify::new(&handle, Owner::on_change).expect("BUG: a live handle binds the Notify");

        notify.send(1);
        widget::drain();

        assert_eq!(owner.with(|owner| owner.seen.clone()), [1]);
    }

    #[wasm_test]
    #[should_panic(expected = "driven outside an SDK export")]
    fn a_send_outside_an_export_is_a_bug() {
        let owner = owner_with_source();
        let notify = Notify::new(&owner, Owner::on_change);
        widget::leave();

        notify.send(1);
    }
}
