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

use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::rc::Rc;

use bmc_wasm_host::main_loop::{
    ControlFirstPostPoll, ControlFirstSlot, FatalError, process_control_sockets,
    run_control_first_post_poll,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Control(&'static str),
    Disconnect(&'static str),
    PendingControl,
    Accept,
    PendingAdmission,
    Request(&'static str),
    Commit(&'static str),
}

struct TestSlot {
    name: &'static str,
    control: UnixStream,
    wayland: UnixStream,
    events: Rc<RefCell<Vec<Event>>>,
    shutdown_fails: bool,
}

impl TestSlot {
    fn new(name: &'static str, events: Rc<RefCell<Vec<Event>>>) -> (Self, UnixStream, UnixStream) {
        let (control, control_peer) = UnixStream::pair().expect("BUG: control socketpair");
        control
            .set_nonblocking(true)
            .expect("BUG: set control socket nonblocking");
        let (wayland, wayland_peer) = UnixStream::pair().expect("BUG: Wayland socketpair");
        (
            Self {
                name,
                control,
                wayland,
                events,
                shutdown_fails: false,
            },
            control_peer,
            wayland_peer,
        )
    }

    fn request(&mut self) {
        self.events.borrow_mut().push(Event::Request(self.name));
        self.wayland
            .write_all(b"r")
            .expect("BUG: healthy Wayland request must remain writable");
    }

    fn commit(&mut self) {
        self.events.borrow_mut().push(Event::Commit(self.name));
        self.wayland
            .write_all(b"c")
            .expect("BUG: healthy Wayland commit must remain writable");
    }
}

impl ControlFirstSlot for TestSlot {
    fn dispatch_control(&mut self) -> anyhow::Result<()> {
        self.events.borrow_mut().push(Event::Control(self.name));
        let mut byte = [0_u8; 1];
        match (&self.control).read(&mut byte) {
            Ok(0) => anyhow::bail!("control EOF"),
            Ok(_) => anyhow::bail!("unexpected control byte"),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn shutdown_wayland(&self) -> anyhow::Result<()> {
        self.events.borrow_mut().push(Event::Disconnect(self.name));
        if self.shutdown_fails {
            anyhow::bail!("injected Wayland shutdown failure");
        }
        self.wayland.shutdown(Shutdown::Both)?;
        Ok(())
    }
}

struct TestPostPoll {
    slots: Vec<(u64, TestSlot)>,
    disconnected: Vec<u64>,
    events: Rc<RefCell<Vec<Event>>>,
}

impl TestPostPoll {
    fn new(events: Rc<RefCell<Vec<Event>>>, slots: Vec<(u64, TestSlot)>) -> Self {
        Self {
            slots,
            disconnected: Vec::new(),
            events,
        }
    }

    fn run(&mut self) -> Result<(), FatalError> {
        run_control_first_post_poll(self)
    }
}

impl ControlFirstPostPoll for TestPostPoll {
    type Error = FatalError;

    fn process_established_controls(&mut self) -> Result<(), Self::Error> {
        let disconnected =
            process_control_sockets(self.slots.iter_mut().map(|(id, slot)| (*id, slot)));
        self.slots.retain(|(id, _)| !disconnected.contains(id));
        self.disconnected = disconnected;
        Ok(())
    }

    fn process_pending_controls(&mut self) -> Result<(), Self::Error> {
        self.events.borrow_mut().push(Event::PendingControl);
        Ok(())
    }

    fn process_listener(&mut self) -> Result<(), Self::Error> {
        self.events.borrow_mut().push(Event::Accept);
        Ok(())
    }

    fn process_pending_admissions(&mut self) -> Result<(), Self::Error> {
        self.events.borrow_mut().push(Event::PendingAdmission);
        Ok(())
    }

    fn process_widgets(&mut self) -> Result<(), Self::Error> {
        for (_, slot) in &mut self.slots {
            slot.request();
            slot.commit();
        }
        Ok(())
    }
}

#[test]
fn predecessor_eof_precedes_successor_accept_and_wayland_work() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (predecessor, predecessor_control, _predecessor_wayland) =
        TestSlot::new("predecessor", Rc::clone(&events));
    let (sibling, _sibling_control, _sibling_wayland) =
        TestSlot::new("sibling", Rc::clone(&events));
    drop(predecessor_control);

    let mut post_poll = TestPostPoll::new(events.clone(), vec![(1, predecessor), (2, sibling)]);
    post_poll
        .run()
        .expect("BUG: control-first post-poll cycle must succeed");

    assert_eq!(post_poll.disconnected, [1]);
    assert_eq!(
        *events.borrow(),
        [
            Event::Control("predecessor"),
            Event::Disconnect("predecessor"),
            Event::Control("sibling"),
            Event::PendingControl,
            Event::Accept,
            Event::PendingAdmission,
            Event::Request("sibling"),
            Event::Commit("sibling"),
        ],
        "all existing controls and the predecessor disconnect must finish before accept or Wayland work"
    );
}

#[test]
fn eof_closes_only_its_exact_wayland_connection_before_request_and_commit() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (predecessor, predecessor_control, mut predecessor_wayland) =
        TestSlot::new("predecessor", Rc::clone(&events));
    let (sibling, _sibling_control, mut sibling_wayland) =
        TestSlot::new("sibling", Rc::clone(&events));
    drop(predecessor_control);

    let mut post_poll = TestPostPoll::new(events.clone(), vec![(1, predecessor), (2, sibling)]);
    post_poll
        .run()
        .expect("BUG: control-first post-poll cycle must succeed");
    assert_eq!(post_poll.disconnected, [1]);

    let mut byte = [0_u8; 1];
    assert_eq!(
        predecessor_wayland
            .read(&mut byte)
            .expect("BUG: shut-down Wayland peer must return EOF"),
        0,
        "control EOF must close the predecessor's exact Wayland transport"
    );

    let mut bytes = [0_u8; 2];
    sibling_wayland
        .read_exact(&mut bytes)
        .expect("BUG: healthy sibling Wayland transport must remain connected");
    assert_eq!(bytes, *b"rc");
    assert_eq!(
        *events.borrow(),
        [
            Event::Control("predecessor"),
            Event::Disconnect("predecessor"),
            Event::Control("sibling"),
            Event::PendingControl,
            Event::Accept,
            Event::PendingAdmission,
            Event::Request("sibling"),
            Event::Commit("sibling"),
        ],
        "the predecessor disconnect must precede every surviving request and commit"
    );
}

#[test]
fn healthy_sibling_progresses_when_another_control_socket_closes() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (closed, closed_control, _closed_wayland) = TestSlot::new("closed", Rc::clone(&events));
    let (healthy, _healthy_control, mut healthy_wayland) =
        TestSlot::new("healthy", Rc::clone(&events));
    drop(closed_control);

    let mut post_poll = TestPostPoll::new(events, vec![(1, closed), (2, healthy)]);
    post_poll
        .run()
        .expect("BUG: healthy sibling cycle must succeed");

    assert_eq!(post_poll.disconnected, [1]);
    let mut bytes = [0_u8; 2];
    healthy_wayland
        .read_exact(&mut bytes)
        .expect("BUG: healthy sibling must still make Wayland progress");
    assert_eq!(bytes, *b"rc");
}

#[test]
fn failed_wayland_shutdown_does_not_block_healthy_widget_work() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (mut closed, closed_control, _closed_wayland) = TestSlot::new("closed", Rc::clone(&events));
    let (healthy, _healthy_control, mut healthy_wayland) =
        TestSlot::new("healthy", Rc::clone(&events));
    closed.shutdown_fails = true;
    drop(closed_control);

    let mut post_poll = TestPostPoll::new(events.clone(), vec![(1, closed), (2, healthy)]);
    post_poll
        .run()
        .expect("a failed shutdown must remain local to its disconnected slot");
    assert_eq!(post_poll.disconnected, [1]);
    let mut bytes = [0_u8; 2];
    healthy_wayland
        .read_exact(&mut bytes)
        .expect("BUG: healthy sibling must still make Wayland progress");
    assert_eq!(bytes, *b"rc");
    assert_eq!(
        *events.borrow(),
        [
            Event::Control("closed"),
            Event::Disconnect("closed"),
            Event::Control("healthy"),
            Event::PendingControl,
            Event::Accept,
            Event::PendingAdmission,
            Event::Request("healthy"),
            Event::Commit("healthy"),
        ],
        "a failed disconnect must not prevent listener admission or healthy widget work"
    );
}
