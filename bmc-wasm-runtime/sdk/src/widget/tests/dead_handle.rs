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

//! A form through a dead handle must leave the host nothing to hold: a resource opened for it
//! would have no handle left to close it, and would count against the widget's limit for good.
//! Each test calls a form through a dead handle more often than the host's default limit
//! allows, then through a live part, which the host must still have room for.

use super::super::{Building, HandleError, Part, PartError, PartHandle, RegisterError};
use crate::mdns::{MdnsBrowse, MdnsEvent};
use crate::net::{FetchRequest, FetchResponse};
use crate::slot::{BitmapSlot, DecodeOutcome, Fit};
use crate::socket::{Socket, SocketEvent};
use crate::ssdp::{SsdpEvent, SsdpSearch};
use crate::timer::Timer;
use crate::udp_broadcast::{UdpBroadcast, UdpBroadcastEvent};
use crate::ws::{Ws, WsEvent};
use crate::{ImageJobId, wasm_test};

/// Above every default limit of the resources below, the largest being 16 fetches.
const PAST_EVERY_LIMIT: usize = 17;

struct Probe;

#[expect(clippy::unused_self, reason = "the SDK calls these as methods")]
impl Probe {
    fn on_tick(&mut self, _timer: &Timer, _elapsed_periods: u32) {}
    fn on_response(&mut self, _response: &FetchResponse) {}
    fn on_ws(&mut self, _ws: &Ws, _event: &WsEvent<'_>) {}
    fn on_socket(&mut self, _socket: &Socket, _event: &SocketEvent<'_>) {}
    fn on_browse(&mut self, _browse: &MdnsBrowse, _event: &MdnsEvent<'_>) {}
    fn on_search(&mut self, _search: &SsdpSearch, _event: &SsdpEvent<'_>) {}
    fn on_broadcast(&mut self, _broadcast: &UdpBroadcast, _event: &UdpBroadcastEvent<'_>) {}
    fn on_decoded(&mut self, _job: ImageJobId, _decoded: DecodeOutcome) {}
}

fn dead_handle() -> PartHandle<Probe> {
    let mut handle = None;
    drop(Part::new_cyclic(|me: &Building<Probe>| {
        handle = Some(me.handle());
        Probe
    }));
    handle.expect("BUG: the build ran")
}

/// Calls `call_dead` through a dead handle past every limit,
/// then checks that `call_live` still succeeds through a live part.
fn assert_live_call_succeeds_after_dead_ones<X, E>(
    call_dead: impl Fn(&PartHandle<Probe>) -> Result<X, HandleError<E>>,
    call_live: impl Fn(&Part<Probe>) -> Result<X, RegisterError<E>>,
) {
    let dead = dead_handle();
    for _ in 0..PAST_EVERY_LIMIT {
        assert!(
            matches!(call_dead(&dead), Err(HandleError::Part(PartError::NotLive))),
            "a dead handle registers nothing"
        );
    }
    let live = Part::new(Probe);
    let held: Vec<Result<X, RegisterError<E>>> =
        (0..PAST_EVERY_LIMIT).map(|_| call_live(&live)).collect();
    assert!(
        held.last().is_some_and(Result::is_err),
        "the host refuses this form within PAST_EVERY_LIMIT calls, or the dead calls prove nothing"
    );
    assert!(
        held.first().is_some_and(Result::is_ok),
        "the calls through the dead handle used up the host's limit"
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_timer() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| Timer::start(dead, 1_000, Probe::on_tick),
        |live| Timer::start(live, 1_000, Probe::on_tick),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_fetch() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| FetchRequest::get("http://example.com/").send_on(dead, Probe::on_response),
        |live| FetchRequest::get("http://example.com/").send_on(live, Probe::on_response),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_scheduled_fetch() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| {
            FetchRequest::get("http://example.com/").send_after_on(dead, 1_000, Probe::on_response)
        },
        |live| {
            FetchRequest::get("http://example.com/").send_after_on(live, 1_000, Probe::on_response)
        },
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_websocket() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| Ws::connect(dead, "ws://example.com/", None, Probe::on_ws),
        |live| Ws::connect(live, "ws://example.com/", None, Probe::on_ws),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_tls_socket() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| Socket::tls_connect(dead, "example.com", 443, Probe::on_socket),
        |live| Socket::tls_connect(live, "example.com", 443, Probe::on_socket),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_insecure_tls_socket() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| Socket::tls_connect_insecure(dead, "example.com", 443, Probe::on_socket),
        |live| Socket::tls_connect_insecure(live, "example.com", 443, Probe::on_socket),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_tcp_socket() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| Socket::tcp_connect(dead, "example.com", 80, Probe::on_socket),
        |live| Socket::tcp_connect(live, "example.com", 80, Probe::on_socket),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_mdns_browse() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| MdnsBrowse::start(dead, &["_http._tcp"], Probe::on_browse),
        |live| MdnsBrowse::start(live, &["_http._tcp"], Probe::on_browse),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_ssdp_search() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| SsdpSearch::start(dead, "ssdp:all", 3, Probe::on_search),
        |live| SsdpSearch::start(live, "ssdp:all", 3, Probe::on_search),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_udp_broadcast() {
    assert_live_call_succeeds_after_dead_ones(
        |dead| UdpBroadcast::start(dead, 4028, "ping", 3, Probe::on_broadcast),
        |live| UdpBroadcast::start(live, 4028, "ping", 3, Probe::on_broadcast),
    );
}

#[wasm_test]
fn a_dead_handle_leaks_no_decode() {
    const SLOT: BitmapSlot = BitmapSlot::new("probe");
    let fit = Fit::Within { w: 16, h: 16 };
    assert_live_call_succeeds_after_dead_ones(
        |dead| SLOT.set_fit_on(dead, b"not an image", fit, b"probe", Probe::on_decoded),
        |live| SLOT.set_fit_on(live, b"not an image", fit, b"probe", Probe::on_decoded),
    );
}
