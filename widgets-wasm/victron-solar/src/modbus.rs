// Copyright (C) 2026  Jan Novák
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Modbus TCP reading of a Venus OS device: read holding registers
//! (function 3) only, one request in flight at a time.
//!
//! Unit 100 is the system service (serial number, live PV power). Solar
//! chargers and PV inverters each sit on a unit id that depends on how they
//! are connected, so [`Poller`] finds them by asking every unit id once per
//! connection (about 0.3 s on a Cerbo GX), then reads them every round.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;

use crate::venus::Readings;

pub const STANDARD_PORT: u16 = 502;

const SYSTEM_UNIT: u8 = 100;
const LAST_UNIT: u8 = 247;

/// Serial number, 6 registers of ASCII: the id MQTT calls the portal id.
const REG_SERIAL: (u16, u16) = (800, 6);
const REG_DC_PV_POWER: (u16, u16) = (850, 1);
/// PV power on AC output, grid and genset, L1..L3 each, in W.
const REG_AC_PV_POWER: (u16, u16) = (808, 9);
const AC_PV_KEYS: [&str; 9] = [
    "PvOnOutput/L1",
    "PvOnOutput/L2",
    "PvOnOutput/L3",
    "PvOnGrid/L1",
    "PvOnGrid/L2",
    "PvOnGrid/L3",
    "PvOnGenset/L1",
    "PvOnGenset/L2",
    "PvOnGenset/L3",
];
/// Solar charger: yield today, max power today, yield yesterday, max power
/// yesterday, error code, PV power, user yield. Yields in 0.1 kWh.
const REG_CHARGER: (u16, u16) = (784, 7);
/// PV inverter: forward energy of L1 as u32 in 0.01 kWh; L2 and L3 follow,
/// two registers apart. Read per phase, since single-phase inverters lack
/// the other two.
const REG_PVINVERTER_ENERGY: u16 = 1046;

/// Read `count` holding registers from `reg` on `unit`.
#[must_use]
pub fn read_request(tid: u16, unit: u8, reg: u16, count: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&tid.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // protocol: Modbus
    out.extend_from_slice(&6u16.to_be_bytes()); // unit + function + reg + count
    out.push(unit);
    out.push(3);
    out.extend_from_slice(&reg.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out
}

#[derive(Debug, PartialEq)]
pub struct Response {
    pub tid: u16,
    /// The registers, or the exception code (10/11: no such unit).
    pub result: Result<Vec<u16>, u8>,
}

/// Reassembles responses from a byte stream that arrives in arbitrary chunks.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Next complete response, or `None` until more bytes arrive.
    pub fn next_response(&mut self) -> Option<Response> {
        loop {
            if self.buf.len() < 7 {
                return None;
            }
            let tid = u16::from_be_bytes([self.buf[0], self.buf[1]]);
            let len = usize::from(u16::from_be_bytes([self.buf[4], self.buf[5]]));
            // The length counts the unit byte plus a PDU of at most 253 bytes;
            // anything else means the stream is out of step and can't recover.
            if !(2..=254).contains(&len) {
                self.buf.clear();
                return None;
            }
            let total = 6 + len;
            if self.buf.len() < total {
                return None;
            }
            let pdu: Vec<u8> = self.buf[7..total].to_vec();
            self.buf.drain(..total);
            if let Some(result) = parse_pdu(&pdu) {
                return Some(Response { tid, result });
            }
        }
    }
}

/// `None` for anything that isn't a function-3 reply, which is skipped.
fn parse_pdu(pdu: &[u8]) -> Option<Result<Vec<u16>, u8>> {
    match *pdu.first()? {
        0x03 => {
            let n = usize::from(*pdu.get(1)?);
            let data = pdu.get(2..2 + n)?;
            let (pairs, _) = data.as_chunks::<2>();
            Some(Ok(pairs
                .iter()
                .map(|&pair| u16::from_be_bytes(pair))
                .collect()))
        }
        0x83 => Some(Err(*pdu.get(1)?)),
        _ => None,
    }
}

/// Unwraps the solar chargers' 16-bit user-yield registers (0.1 kWh, so they
/// roll over at 6553.5 kWh) into a lifetime count that keeps rising. The
/// caller persists it so a rollover while the widget isn't running is still
/// caught.
#[derive(Default, Debug, PartialEq)]
pub struct YieldCounters {
    /// unit → (last raw value, rollovers seen)
    units: BTreeMap<u8, (u16, u32)>,
    /// Worth saving since the caller last did: a charger seen for the first
    /// time, a rollover or reset, or another 10 kWh. The saved value then
    /// stays close enough to catch a rollover while the widget isn't running.
    pub dirty: bool,
}

/// A drop is a rollover when the counter would have covered less than this
/// (655 kWh, weeks of yield even for a large system) going forward across
/// 65535 → 0. A bigger drop is someone resetting the counter, which the
/// ledger handles like any other reset.
const ROLLOVER_WINDOW: u32 = 6553;

impl YieldCounters {
    /// Lifetime yield of `unit` in kWh.
    pub fn lifetime_kwh(&mut self, unit: u8, raw: u16) -> f64 {
        let (last, mut wraps) = self.units.get(&unit).copied().unwrap_or_else(|| {
            self.dirty = true;
            (raw, 0)
        });
        if raw / 100 != last / 100 {
            self.dirty = true;
        }
        if raw < last {
            if 65536 - u32::from(last) + u32::from(raw) < ROLLOVER_WINDOW {
                wraps += 1;
            } else {
                wraps = 0;
            }
        }
        self.units.insert(unit, (raw, wraps));
        (f64::from(wraps) * 65536.0 + f64::from(raw)) / 10.0
    }

    /// `"v1 224:48840:0 225:12:1"`
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("v1");
        for (unit, (last, wraps)) in &self.units {
            let _ = write!(out, " {unit}:{last}:{wraps}");
        }
        out
    }

    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        if parts.next()? != "v1" {
            return None;
        }
        let mut units = BTreeMap::new();
        for part in parts {
            let mut f = part.split(':');
            let unit = f.next()?.parse().ok()?;
            let last = f.next()?.parse().ok()?;
            let wraps = f.next()?.parse().ok()?;
            units.insert(unit, (last, wraps));
        }
        Some(Self {
            units,
            dirty: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Query {
    Serial,
    FindCharger(u8),
    FindPvInverter(u8),
    DcPvPower,
    AcPvPower,
    Charger(u8),
    PvInverterPhase(u8, u16),
}

impl Query {
    /// (unit, first register, register count)
    fn target(self) -> (u8, u16, u16) {
        match self {
            Self::Serial => (SYSTEM_UNIT, REG_SERIAL.0, REG_SERIAL.1),
            Self::FindCharger(unit) | Self::Charger(unit) => (unit, REG_CHARGER.0, REG_CHARGER.1),
            Self::FindPvInverter(unit) => (unit, REG_PVINVERTER_ENERGY, 2),
            Self::DcPvPower => (SYSTEM_UNIT, REG_DC_PV_POWER.0, REG_DC_PV_POWER.1),
            Self::AcPvPower => (SYSTEM_UNIT, REG_AC_PV_POWER.0, REG_AC_PV_POWER.1),
            Self::PvInverterPhase(unit, phase) => (unit, REG_PVINVERTER_ENERGY + 2 * phase, 2),
        }
    }
}

/// Which batch of queries is queued.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Serial,
    Scan,
    Poll,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// The device's serial number, the same id MQTT reports as portal id.
    Portal(String),
    Found {
        chargers: usize,
        pv_inverters: usize,
    },
}

#[derive(Default, Debug)]
pub struct Step {
    /// The next request, to write to the socket.
    pub send: Option<Vec<u8>>,
    pub event: Option<Event>,
    /// Something the widget shows may have changed.
    pub changed: bool,
}

/// Requests kept in flight at once. Replies reach the widget in host ticks
/// (~70 ms apart on the Deck), so one at a time made the 494-request unit
/// scan take over 30 s; Venus OS answers pipelined requests in order.
const WINDOW: usize = 32;

/// Drives one connection: serial number, then the unit scan, then a round
/// of reads each time [`Poller::request_round`] asks for one.
#[derive(Default)]
pub struct Poller {
    next_tid: u16,
    in_flight: BTreeMap<u16, Query>,
    queue: VecDeque<Query>,
    phase: Phase,
    portal: Option<String>,
    scanned: bool,
    round_wanted: bool,
    chargers: Vec<u8>,
    pv_inverters: Vec<u8>,
    pv_inverter_phases_kwh: BTreeMap<u8, [Option<f64>; 3]>,
    pub yield_counters: YieldCounters,
}

impl Poller {
    /// First request once the socket is up; the first round follows the scan.
    pub fn start(&mut self) -> Option<Vec<u8>> {
        self.round_wanted = true;
        self.advance(&mut Readings::default()).0
    }

    /// Ask for a fresh round of reads. Returns the requests to send if the
    /// connection was idle; a round still running is left to finish.
    pub fn request_round(&mut self, readings: &mut Readings) -> Option<Vec<u8>> {
        self.round_wanted = true;
        self.advance(readings).0
    }

    pub fn on_response(&mut self, response: Response, readings: &mut Readings) -> Step {
        let mut step = Step::default();
        // No match: a reply to a request from before a reconnect, not ours.
        let Some(query) = self.in_flight.remove(&response.tid) else {
            return step;
        };
        (step.changed, step.event) = self.apply(query, response.result, readings);
        let (send, finished) = self.advance(readings);
        step.send = send;
        match finished {
            Phase::Scan => {
                step.event = Some(Event::Found {
                    chargers: self.chargers.len(),
                    pv_inverters: self.pv_inverters.len(),
                });
            }
            Phase::Poll => step.changed = true,
            Phase::Idle | Phase::Serial => {}
        }
        step
    }

    /// Fill the window with queued queries, queuing the next batch once
    /// this one has run out and all its replies are in. Also returns the
    /// batch that just finished, if one did.
    fn advance(&mut self, readings: &mut Readings) -> (Option<Vec<u8>>, Phase) {
        let mut finished = Phase::Idle;
        if self.queue.is_empty() && self.in_flight.is_empty() {
            finished = core::mem::take(&mut self.phase);
            if finished == Phase::Poll {
                readings.complete = true;
            }
            self.phase = self.refill();
        }
        let mut out = Vec::new();
        while self.in_flight.len() < WINDOW
            && let Some(query) = self.queue.pop_front()
        {
            self.next_tid = self.next_tid.wrapping_add(1);
            self.in_flight.insert(self.next_tid, query);
            let (unit, reg, count) = query.target();
            out.extend(read_request(self.next_tid, unit, reg, count));
        }
        ((!out.is_empty()).then_some(out), finished)
    }

    fn refill(&mut self) -> Phase {
        if self.portal.is_none() {
            self.queue.push_back(Query::Serial);
            Phase::Serial
        } else if !self.scanned {
            self.scanned = true;
            let units = (1..=LAST_UNIT).filter(|&u| u != SYSTEM_UNIT);
            self.queue.extend(units.clone().map(Query::FindCharger));
            self.queue.extend(units.map(Query::FindPvInverter));
            Phase::Scan
        } else if self.round_wanted {
            self.round_wanted = false;
            self.queue.extend([Query::DcPvPower, Query::AcPvPower]);
            self.queue
                .extend(self.chargers.iter().map(|&u| Query::Charger(u)));
            for &unit in &self.pv_inverters {
                self.queue
                    .extend((0..3).map(|phase| Query::PvInverterPhase(unit, phase)));
            }
            Phase::Poll
        } else {
            Phase::Idle
        }
    }

    /// Fold one reply into the readings: (changed, event).
    fn apply(
        &mut self,
        query: Query,
        result: Result<Vec<u16>, u8>,
        readings: &mut Readings,
    ) -> (bool, Option<Event>) {
        let regs = result.ok();
        match query {
            Query::Serial => {
                let serial = regs.map(|r| ascii(&r)).filter(|s| !s.is_empty());
                // Without a serial the totals still work, just under a shared key.
                let serial = serial.unwrap_or_else(|| "unknown".to_owned());
                self.portal = Some(serial.clone());
                return (false, Some(Event::Portal(serial)));
            }
            Query::FindCharger(unit) => {
                if regs.is_some_and(|r| r.len() == usize::from(REG_CHARGER.1)) {
                    self.chargers.push(unit);
                }
                return (false, None);
            }
            Query::FindPvInverter(unit) => {
                if regs.is_some_and(|r| r.len() == 2) {
                    self.pv_inverters.push(unit);
                }
                return (false, None);
            }
            Query::DcPvPower => {
                readings.set_dc_pv_w(regs.and_then(|r| r.first().copied()).map(f64::from));
            }
            Query::AcPvPower => {
                for (i, key) in AC_PV_KEYS.iter().enumerate() {
                    let w = regs.as_ref().and_then(|r| r.get(i).copied());
                    readings.set_ac_pv_w(key, w.map(f64::from));
                }
            }
            Query::Charger(unit) => match regs {
                Some(r) if r.len() == usize::from(REG_CHARGER.1) => {
                    let inst = u32::from(unit);
                    readings.set_charger_day_kwh(inst, 0, Some(f64::from(r[0]) / 10.0));
                    readings.set_charger_day_kwh(inst, 1, Some(f64::from(r[2]) / 10.0));
                    let lifetime = self.yield_counters.lifetime_kwh(unit, r[6]);
                    readings.set_charger_lifetime_kwh(inst, Some(lifetime));
                }
                _ => readings.remove_charger(u32::from(unit)),
            },
            Query::PvInverterPhase(unit, phase) => {
                let kwh = regs
                    .filter(|r| r.len() == 2)
                    .map(|r| f64::from((u32::from(r[0]) << 16) | u32::from(r[1])) / 100.0);
                let phases = self.pv_inverter_phases_kwh.entry(unit).or_default();
                phases[usize::from(phase)] = kwh;
                let total = phases.iter().flatten().copied().reduce(|a, b| a + b);
                readings.set_pvinverter_lifetime_kwh(u32::from(unit), total);
            }
        }
        (true, None)
    }
}

/// Two characters per register, high byte first; padding NULs dropped.
fn ascii(regs: &[u16]) -> String {
    regs.iter()
        .flat_map(|r| r.to_be_bytes())
        .filter(|&b| b != 0)
        .map(char::from)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    fn reply(tid: u16, unit: u8, regs: &[u16]) -> Vec<u8> {
        let mut pdu = vec![0x03, u8::try_from(regs.len() * 2).unwrap()];
        for r in regs {
            pdu.extend_from_slice(&r.to_be_bytes());
        }
        frame(tid, unit, &pdu)
    }

    fn exception(tid: u16, unit: u8, code: u8) -> Vec<u8> {
        frame(tid, unit, &[0x83, code])
    }

    fn frame(tid: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&tid.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&u16::try_from(pdu.len() + 1).unwrap().to_be_bytes());
        out.push(unit);
        out.extend_from_slice(pdu);
        out
    }

    /// (tid, unit, reg, count) of a request.
    fn parse_request(req: &[u8]) -> (u16, u8, u16, u16) {
        assert_eq!(req.len(), 12);
        assert_eq!(req[7], 3, "function 3 only");
        (
            u16::from_be_bytes([req[0], req[1]]),
            req[6],
            u16::from_be_bytes([req[8], req[9]]),
            u16::from_be_bytes([req[10], req[11]]),
        )
    }

    #[wasm_test]
    fn read_request_is_a_function_3_mbap_frame() {
        assert_eq!(
            read_request(0x0102, 224, 784, 7),
            [1, 2, 0, 0, 0, 6, 224, 3, 0x03, 0x10, 0, 7]
        );
    }

    #[wasm_test]
    fn decodes_replies_and_exceptions_across_chunks() {
        let mut wire = reply(1, 100, &[0x3031, 0x3233]);
        wire.extend(exception(2, 7, 10));
        let mut d = Decoder::default();
        let mut got = Vec::new();
        for chunk in wire.chunks(3) {
            d.push(chunk);
            while let Some(r) = d.next_response() {
                got.push(r);
            }
        }
        assert_eq!(
            got,
            [
                Response {
                    tid: 1,
                    result: Ok(vec![0x3031, 0x3233])
                },
                Response {
                    tid: 2,
                    result: Err(10)
                },
            ]
        );
    }

    #[wasm_test]
    fn a_nonsense_length_drops_the_buffer() {
        let mut d = Decoder::default();
        d.push(&[0, 1, 0, 0, 0xFF, 0xFF, 1, 3, 0]);
        assert_eq!(d.next_response(), None);
        d.push(&reply(5, 100, &[42]));
        assert_eq!(d.next_response().unwrap().result, Ok(vec![42]));
    }

    #[wasm_test]
    fn serial_is_ascii_without_padding() {
        assert_eq!(
            ascii(&[0x3031, 0x3233, 0x3435, 0x3637, 0x3839, 0x6162]),
            "0123456789ab"
        );
        assert_eq!(ascii(&[0x6162, 0x0000]), "ab");
    }

    #[wasm_test]
    fn yield_counter_rolls_over_but_a_reset_starts_again() {
        let mut c = YieldCounters::default();
        assert!((c.lifetime_kwh(224, 65_000) - 6500.0).abs() < 1e-9);
        assert!(c.dirty, "a new charger is saved");
        c.dirty = false;
        c.lifetime_kwh(224, 65_090);
        assert!(!c.dirty, "less than 10 kWh on isn't");
        // 65090 → 300 is 74.6 kWh forward across the top: a rollover.
        assert!((c.lifetime_kwh(224, 300) - 6583.6).abs() < 1e-9);
        assert!(c.dirty);
        c.dirty = false;
        c.lifetime_kwh(224, 400);
        assert!(c.dirty, "another 10 kWh is");
        // 30000 → 10 would be ~3550 kWh forward: someone reset it.
        let mut c = YieldCounters::default();
        c.lifetime_kwh(1, 30_000);
        assert!((c.lifetime_kwh(1, 10) - 1.0).abs() < 1e-9);
    }

    #[wasm_test]
    fn yield_counters_survive_a_restart() {
        let mut c = YieldCounters::default();
        c.lifetime_kwh(224, 65_000);
        c.lifetime_kwh(224, 300);
        c.lifetime_kwh(9, 7);
        let text = c.to_text();
        assert_eq!(text, "v1 9:7:0 224:300:1");
        let mut back = YieldCounters::from_text(&text).unwrap();
        assert!((back.lifetime_kwh(224, 310) - 6584.6).abs() < 1e-9);
        assert_eq!(YieldCounters::from_text("v0 1:2:3"), None);
        assert_eq!(YieldCounters::from_text("v1 1:x:3"), None);
    }

    /// A simulated device: one charger on unit 224, one PV inverter on
    /// unit 20 reporting only L1, nothing else.
    fn device(unit: u8, reg: u16, tid: u16) -> Vec<u8> {
        match (unit, reg) {
            (100, 800) => reply(tid, unit, &[0x3031, 0x3233, 0x3435, 0x3637, 0x3839, 0x6162]),
            (100, 850) => reply(tid, unit, &[120]),
            (100, 808) => reply(tid, unit, &[0, 0, 0, 30, 0, 0, 0, 0, 0]),
            (224, 784) => reply(tid, unit, &[14, 1223, 11, 355, 0, 0, 48_840]),
            (20, 1046) => reply(tid, unit, &[0, 600]),
            (20, _) => exception(tid, unit, 2),
            _ => exception(tid, unit, 10),
        }
    }

    /// The requests in what the poller sent.
    fn requests(bytes: &[u8]) -> Vec<(u16, u8, u16, u16)> {
        bytes.chunks(12).map(parse_request).collect()
    }

    /// Answer requests in order until the poller goes quiet, checking it
    /// never has more than [`WINDOW`] out; returns the events and the units
    /// asked.
    fn run_with(
        poller: &mut Poller,
        first: Option<Vec<u8>>,
        readings: &mut Readings,
        answer: impl Fn(u8, u16, u16) -> Vec<u8>,
    ) -> (Vec<Event>, Vec<u8>) {
        let mut events = Vec::new();
        let mut units = Vec::new();
        let mut out: VecDeque<_> = first.map(|b| requests(&b)).unwrap_or_default().into();
        while let Some((tid, unit, reg, _)) = out.pop_front() {
            units.push(unit);
            assert!(units.len() < 2000, "runaway");
            let mut d = Decoder::default();
            d.push(&answer(unit, reg, tid));
            let step = poller.on_response(d.next_response().unwrap(), readings);
            events.extend(step.event);
            out.extend(step.send.map(|b| requests(&b)).unwrap_or_default());
            assert!(out.len() <= WINDOW);
        }
        (events, units)
    }

    fn run(poller: &mut Poller, first: Option<Vec<u8>>, readings: &mut Readings) -> Vec<Event> {
        run_with(poller, first, readings, device).0
    }

    #[wasm_test]
    fn finds_the_serial_and_solar_devices_then_reads_them() {
        let mut p = Poller::default();
        let mut r = Readings::default();
        let first = p.start();
        assert_eq!(parse_request(first.as_ref().unwrap()).1, 100);
        let events = run(&mut p, first, &mut r);
        assert_eq!(
            events,
            [
                Event::Portal("0123456789ab".into()),
                Event::Found {
                    chargers: 1,
                    pv_inverters: 1
                },
            ]
        );
        assert!(r.complete, "a full round has been read");
        assert!((r.pv_power_w().unwrap() - 150.0).abs() < 1e-9);
        assert!((r.lifetime_kwh().unwrap() - 4890.0).abs() < 1e-9);
        assert_eq!(r.daily_history_kwh(), vec![1.4, 1.1]);

        // The next round skips serial and scan: just the six reads, sent
        // together.
        let round = p.request_round(&mut r);
        assert_eq!(round.as_ref().map(|b| requests(b).len()), Some(6));
        let (_, units) = run_with(&mut p, round, &mut r, device);
        assert_eq!(units, [100, 100, 224, 20, 20, 20]);
    }

    #[wasm_test]
    fn the_scan_is_pipelined() {
        let mut p = Poller::default();
        let mut r = Readings::default();
        let serial = p.start().unwrap();
        let (tid, unit, reg, _) = parse_request(&serial);
        let mut d = Decoder::default();
        d.push(&device(unit, reg, tid));
        let scan = p.on_response(d.next_response().unwrap(), &mut r).send;
        assert_eq!(scan.map(|b| requests(&b).len()), Some(WINDOW));
    }

    #[wasm_test]
    fn a_round_already_running_is_not_restarted_and_stray_replies_are_ignored() {
        let mut p = Poller::default();
        let mut r = Readings::default();
        let first = p.start().unwrap();
        assert_eq!(p.request_round(&mut r), None, "serial still in flight");
        let (tid, ..) = parse_request(&first);
        let mut d = Decoder::default();
        d.push(&reply(tid.wrapping_add(7), 100, &[1]));
        let step = p.on_response(d.next_response().unwrap(), &mut r);
        assert!(step.send.is_none() && step.event.is_none() && !step.changed);
    }

    #[wasm_test]
    fn a_charger_that_stops_answering_drops_out() {
        let mut p = Poller::default();
        let mut r = Readings::default();
        let first = p.start();
        run(&mut p, first, &mut r);
        let round = p.request_round(&mut r);
        run_with(&mut p, round, &mut r, |unit, reg, tid| {
            if unit == 224 {
                exception(tid, unit, 11)
            } else {
                device(unit, reg, tid)
            }
        });
        assert!((r.lifetime_kwh().unwrap() - 6.0).abs() < 1e-9);
        assert!(r.daily_history_kwh().is_empty());
    }
}
