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

//! What an entity event allocates must not grow with the subscription it reaches:
//! under fuel metering an allocation costs a widget about 1,000 fuel,
//! and a subscription follows up to 32 entities with 16 attributes each.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use bmc_wasm_sdk::wasm_test;
use home_assistant::proto::{JsonLookup, Session, Setup};
use serde_json::{Map, Value, json};

struct Counting;

// Per thread, so tests running side by side count only their own allocations.
thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static ALLOCATED_BYTES: Cell<usize> = const { Cell::new(0) };
    static IN_HOST: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: forwards every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn count(bytes: usize) {
    if !IN_HOST.get() {
        ALLOCATIONS.set(ALLOCATIONS.get() + 1);
        ALLOCATED_BYTES.set(ALLOCATED_BYTES.get() + bytes);
    }
}

/// Run `lookup` as the host's parser would, outside the widget's allocations.
fn in_host<T>(lookup: impl FnOnce() -> T) -> T {
    IN_HOST.set(true);
    let found = lookup();
    IN_HOST.set(false);
    found
}

/// A message read the way the host's `JsonDoc` reads it:
/// only a string it hands back is allocated in the widget.
struct HostDoc(Value);

impl JsonLookup for HostDoc {
    fn str(&self, path: &str) -> Option<String> {
        in_host(|| self.0.pointer(path)?.as_str()).map(str::to_owned)
    }

    fn str_max(&self, path: &str, max: usize) -> Option<String> {
        in_host(|| {
            self.0
                .pointer(path)?
                .as_str()
                .filter(|text| text.len() <= max)
        })
        .map(str::to_owned)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        in_host(|| self.0.pointer(path)?.as_i64())
    }

    fn f64(&self, path: &str) -> Option<f64> {
        in_host(|| self.0.pointer(path)?.as_f64())
    }

    fn bool(&self, path: &str) -> Option<bool> {
        in_host(|| self.0.pointer(path)?.as_bool())
    }

    fn len(&self, path: &str) -> Option<usize> {
        in_host(|| match self.0.pointer(path)? {
            Value::Array(items) => Some(items.len()),
            Value::Object(members) => Some(members.len()),
            _ => None,
        })
    }

    fn is_array(&self, path: &str) -> bool {
        in_host(|| matches!(self.0.pointer(path), Some(Value::Array(_))))
    }

    fn exists(&self, path: &str) -> bool {
        in_host(|| self.0.pointer(path).is_some())
    }
}

fn allocations_of(session: &mut Session, message: Value) -> usize {
    let message = HostDoc(message);
    let before = ALLOCATIONS.get();
    let out = session.on_message(&message);
    let allocations = ALLOCATIONS.get() - before;
    drop(out);
    allocations
}

fn bytes_allocated_by(session: &mut Session, message: Value) -> usize {
    let message = HostDoc(message);
    let before = ALLOCATED_BYTES.get();
    let out = session.on_message(&message);
    let bytes = ALLOCATED_BYTES.get() - before;
    drop(out);
    bytes
}

/// A session following `ids`, each with the text attributes `names`, its snapshot applied.
fn following(ids: &[String], names: &[String]) -> Session {
    let mut session = Session::new(Setup {
        websocket_url: "ws://ha.test/api/websocket".to_owned(),
        token: "token".to_owned(),
        reserved: |_| false,
        iso_time: |_| None,
    });
    let _ = session.start();
    let _ = session.on_message(&HostDoc(
        json!({"type": "auth_required", "ha_version": "2026.10.0"}),
    ));
    let _ = session.on_message(&HostDoc(json!({"type": "auth_ok"})));
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let _ = session
        .subscribe(&id_refs, &name_refs)
        .expect("BUG: the test subscription is within bounds");
    let mut added = Map::new();
    for id in ids {
        let values: Map<String, Value> = names
            .iter()
            .map(|name| (name.clone(), json!("a text value")))
            .collect();
        added.insert(id.clone(), json!({"s": "21.5", "a": values, "lc": 1.0}));
    }
    let _ = session.on_message(&HostDoc(
        json!({"id": 1, "type": "event", "event": {"a": added}}),
    ));
    session
}

/// The allocations of one entity's change, and of a change to an entity not followed,
/// in a subscription of `entities` entities tracking `attributes` attributes each.
fn measure(entities: usize, attributes: usize) -> (usize, usize) {
    let ids: Vec<String> = (0..entities).map(|i| format!("sensor.s{i}")).collect();
    let names: Vec<String> = (0..attributes).map(|i| format!("attribute_{i}")).collect();
    let mut session = following(&ids, &names);

    let one_change = allocations_of(
        &mut session,
        json!({"id": 1, "type": "event", "event": {"c": {ids[0].clone(): {"+": {"s": "22.0"}}}}}),
    );
    let unrelated = allocations_of(
        &mut session,
        json!({"id": 1, "type": "event", "event": {"c": {"sensor.other": {"+": {"s": "1"}}}}}),
    );
    (one_change, unrelated)
}

#[wasm_test(
    fuel = 100e6,
    reason = "builds and applies a 32 x 16 subscription snapshot"
)]
fn an_event_allocates_the_same_whatever_the_subscription_follows() {
    let small = measure(1, 1);
    let full = measure(32, 16);

    assert_eq!(
        full, small,
        "(one entity's change, a change to an entity not followed): \
         the entities an event does not name must cost no allocation"
    );
}

#[wasm_test(fuel = 100e6, reason = "builds a 64 KiB attribute value in the guest")]
fn a_text_attribute_past_its_cap_is_never_copied_in() {
    const LONG: usize = 64 * 1024;
    let ids = ["sensor.s0".to_owned()];
    let names = ["note".to_owned()];
    let mut session = following(&ids, &names);
    let change = json!({"id": 1, "type": "event", "event": {"c": {"sensor.s0": {"+": {"a": {"note": "x".repeat(LONG)}}}}}});

    assert!(
        bytes_allocated_by(&mut session, change) < LONG,
        "a string over the cap is refused by its length, before it reaches the widget"
    );
}
