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

//! The exports `#[widget]` generates, called by name as the host calls them.

use core::cell::RefCell;

use crate::{Building, Widget, wasm_test, widget};

/// Each hook that ran, with the renders and the last delta the instance had seen by then.
type Call = (&'static str, usize, Option<u32>);

thread_local! {
    static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default)]
struct Probe {
    renders: usize,
    last_delta_ms: Option<u32>,
}

impl Probe {
    fn note(&self, hook: &'static str) {
        CALLS.with_borrow_mut(|calls| calls.push((hook, self.renders, self.last_delta_ms)));
    }
}

#[widget]
impl Widget for Probe {
    fn init(_: &Building<Self>) -> Self {
        let probe = Self::default();
        probe.note("init");
        probe
    }

    fn render(&mut self, delta_ms: u32) {
        self.renders += 1;
        self.last_delta_ms = Some(delta_ms);
        self.note("render");
    }

    fn on_wake(&mut self) {
        self.note("on_wake");
    }

    fn unload(self) {
        self.note("unload");
    }
}

unsafe extern "C" {
    fn init();
    fn render(delta_ms: u32);
    fn on_wake();
    fn unload();
}

#[wasm_test]
fn each_export_runs_its_hook_on_the_one_instance() {
    // SAFETY: these are the `#[widget]` exports above, called in the order the host calls them.
    unsafe {
        widget::as_host(|| init());
        widget::as_host(|| render(16));
        widget::as_host(|| render(17));
        widget::as_host(|| on_wake());
        widget::as_host(|| unload());
    }

    assert_eq!(
        CALLS.with_borrow(Clone::clone),
        [
            ("init", 0, None),
            ("render", 1, Some(16)),
            ("render", 2, Some(17)),
            ("on_wake", 2, Some(17)),
            ("unload", 2, Some(17)),
        ]
    );
}
