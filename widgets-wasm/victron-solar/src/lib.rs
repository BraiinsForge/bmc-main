// Copyright (C) 2026  Jan Novák
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Victron Solar widget — live PV power and solar yield (today, this month,
//! this year) read straight from the device running Venus OS in a Victron
//! system, over the local network.
//!
//! Three ways in, picked in the settings: MQTT over TLS (the device's
//! certificate is self-signed), plain MQTT, or Modbus TCP. Over MQTT the
//! widget finds the portal id, subscribes to the solar topics and sends the
//! keepalive that makes Venus OS publish them (see [`venus`]); over Modbus
//! it scans for solar devices and polls them (see [`modbus`]). Either way
//! the same [`venus::Readings`] feed [`totals`], which counts month/year.

mod manifest_params;
mod modbus;
mod mqtt;
mod totals;
mod venus;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use std::cell::RefCell;

    use bmc_wasm_sdk::socket::{Socket, SocketEvent, tcp_connect, tls_connect_insecure};
    use bmc_wasm_sdk::timer::{self, Timer};
    use bmc_wasm_sdk::types::Availability;
    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    use super::manifest_params::{self, Connection};
    use super::modbus::{self, Poller, YieldCounters};
    use super::mqtt::{self, Incoming};
    use super::totals::{Date, Ledger, Totals};
    use super::venus::{self, Readings};

    /// Modbus polling cadence; MQTT only pings every [`KEEPALIVE_SECS`].
    const TICK_MS: u32 = 5_000;
    /// MQTT ping + keepalive cadence. Venus OS stops publishing ~60 s after
    /// the last keepalive, and the MQTT session itself expires after
    /// [`mqtt::KEEPALIVE_SECS`].
    const KEEPALIVE_SECS: i64 = 25;
    /// No packet at all for this long: the connection is dead, reconnect.
    const STALE_SECS: i64 = 90;
    /// Sent on every keepalive after the first: refreshes the subscription
    /// without making Venus OS republish every value (older versions ignore
    /// the option and republish, which is merely heavier).
    const KEEPALIVE_QUIET: &[u8] = br#"{"keepalive-options": ["suppress-republish"]}"#;

    const NOT_AVAILABLE: &str = "--";
    const BG_COLOR: Color = BLACK;
    const HEADER_COLOR: Color = GRAY_60;
    const SUN_COLOR: Color = ORANGE_50;

    const SUN_ICON: Svg = include_svg!("assets/sun.svg");

    #[derive(Clone, Copy, PartialEq)]
    enum Status {
        Connecting,
        Live,
        Unreachable,
        Refused(u8),
        /// Modbus found neither a solar charger nor a PV inverter.
        NoSolar,
    }

    #[derive(Clone, Copy)]
    struct SolarData {
        status: Status,
        /// "MQTT" or "Modbus TCP", for the status line.
        service: &'static str,
        pv_power_w: Availability<f64>,
        today_kwh: Availability<f64>,
        month_kwh: Availability<f64>,
        month_since: Option<Date>,
        last_month_kwh: Availability<f64>,
        year_kwh: Availability<f64>,
        year_since: Option<Date>,
        last_year_kwh: Availability<f64>,
    }

    impl Default for SolarData {
        fn default() -> Self {
            Self {
                status: Status::Connecting,
                service: "MQTT",
                pv_power_w: Availability::default(),
                today_kwh: Availability::default(),
                month_kwh: Availability::default(),
                month_since: None,
                last_month_kwh: Availability::default(),
                year_kwh: Availability::default(),
                year_since: None,
                last_year_kwh: Availability::default(),
            }
        }
    }

    enum Link {
        Mqtt(mqtt::Decoder),
        Modbus(modbus::Decoder, Poller),
    }

    impl Default for Link {
        fn default() -> Self {
            Self::Mqtt(mqtt::Decoder::default())
        }
    }

    #[derive(Default)]
    struct Session {
        link: Link,
        /// The device's id (MQTT portal id = serial number); keys what's kept on the Deck.
        portal: Option<String>,
        readings: Readings,
        ledger: Option<Ledger>,
        last_rx_secs: i64,
        last_keepalive_secs: i64,
        status: Option<Status>,
    }

    thread_local! {
        static DATA: RefCell<SolarData> = RefCell::new(SolarData::default());
        static SESSION: RefCell<Session> = RefCell::new(Session::default());
        static SOCKET: RefCell<Option<Socket>> = const { RefCell::new(None) };
        static TICK: RefCell<Option<Timer>> = const { RefCell::new(None) };
    }

    // ---------------------------------------------------------------------
    // Connection
    // ---------------------------------------------------------------------

    fn now_secs() -> i64 {
        SystemTime::now().unix_secs
    }

    /// Local date and hour, in the Deck's configured timezone.
    fn local_now() -> (Date, u32) {
        let now = SystemTime::now();
        let local = SystemTime {
            unix_secs: format::local_unix_secs_or_system(&now, None),
        }
        .utc();
        (
            Date::new(
                i32::from(local.year),
                u32::from(local.month),
                u32::from(local.day),
            ),
            u32::from(local.hour),
        )
    }

    /// What's kept on the Deck goes into the instance's cache bucket on
    /// flash: `kv` is memory-only on the Deck (the host gives it no
    /// directory), so it would be lost on every widget restart.
    fn store_get(key: &str) -> Option<String> {
        cache::read_bytes(key).and_then(|e| String::from_utf8(e.bytes).ok())
    }

    fn store_put(key: &str, text: &str) {
        cache::put(key, b"", text.as_bytes());
    }

    fn ledger_key(portal: &str) -> String {
        fmt!("ledger-{}", portal)
    }

    fn yield_counters_key(portal: &str) -> String {
        fmt!("modbus-yield-{}", portal)
    }

    fn connect() {
        let params = manifest_params::Params::current();
        let host = params.gx_host.trim().to_string();
        // Optional, so widgets placed before these settings existed still start.
        let connection = params.connection.unwrap_or(Connection::MqttTls);
        let (link, service, standard_port) = match connection {
            Connection::MqttTls => (Link::default(), "MQTT", mqtt::TLS_PORT),
            Connection::Mqtt => (Link::default(), "MQTT", mqtt::PLAIN_PORT),
            Connection::Modbus => (
                Link::Modbus(modbus::Decoder::default(), Poller::default()),
                "Modbus TCP",
                modbus::STANDARD_PORT,
            ),
        };
        let port = u16::try_from(params.port.unwrap_or(0))
            .ok()
            .filter(|&p| p != 0)
            .unwrap_or(standard_port);
        SESSION.with_borrow_mut(|s| {
            let ledger = s.ledger.take();
            *s = Session {
                link,
                last_rx_secs: now_secs(),
                ..Session::default()
            };
            s.ledger = ledger;
        });
        DATA.with_borrow_mut(|d| d.service = service);
        set_status(Status::Connecting);
        if host.is_empty() {
            SOCKET.set(None);
            set_status(Status::Unreachable);
            return;
        }
        SOCKET.set(if matches!(connection, Connection::MqttTls) {
            tls_connect_insecure(&host, port, on_socket_event)
        } else {
            tcp_connect(&host, port, on_socket_event)
        });
        if SOCKET.with_borrow(Option::is_none) {
            log_warn!(
                "victron-solar: host refused a connection to {}:{}",
                host,
                port
            );
            set_status(Status::Unreachable);
        } else {
            log_info!(
                "victron-solar: connecting to {}:{} ({})",
                host,
                port,
                service
            );
        }
    }

    fn set_status(status: Status) {
        DATA.with_borrow_mut(|d| d.status = status);
        request_frame();
    }

    fn on_socket_event(socket: &Socket, event: &SocketEvent<'_>) {
        match event {
            SocketEvent::Connected => {
                let first = SESSION.with_borrow_mut(|s| match &mut s.link {
                    Link::Mqtt(_) => {
                        log_info!("victron-solar: connected, logging in to MQTT");
                        // Random, so two widgets on one Deck don't kick each other off the broker.
                        Some(mqtt::connect(&fmt!("bmc-solar-{}", host::random_u32())))
                    }
                    Link::Modbus(_, poller) => {
                        log_info!("victron-solar: connected, reading over Modbus");
                        poller.start()
                    }
                });
                if let Some(first) = first {
                    socket.write(&first);
                }
            }
            SocketEvent::Data(bytes) => on_data(socket, bytes),
            SocketEvent::Closed(code) => {
                log_info!("victron-solar: connection closed ({})", code);
                SOCKET.set(None);
                // A refusal is the more useful thing to show than the close that follows it.
                if !matches!(DATA.with_borrow(|d| d.status), Status::Refused(_)) {
                    set_status(Status::Unreachable);
                }
            }
        }
    }

    fn on_data(socket: &Socket, bytes: &[u8]) {
        let changed = SESSION.with_borrow_mut(|s| {
            s.last_rx_secs = now_secs();
            if matches!(s.link, Link::Mqtt(_)) {
                on_mqtt_data(s, socket, bytes)
            } else {
                on_modbus_data(s, socket, bytes)
            }
        });
        if let Some(status) = SESSION.with_borrow_mut(|s| s.status.take()) {
            set_status(status);
        }
        if changed {
            recompute();
        }
    }

    /// The device answered with its id: pick up what was kept for it.
    fn adopt_portal(s: &mut Session, portal: &str) {
        s.ledger = store_get(&ledger_key(portal)).and_then(|t| Ledger::from_text(&t));
        s.portal = Some(portal.to_string());
        s.status = Some(Status::Live);
    }

    fn on_mqtt_data(s: &mut Session, socket: &Socket, bytes: &[u8]) -> bool {
        let mut changed = false;
        if let Link::Mqtt(decoder) = &mut s.link {
            decoder.push(bytes);
        }
        loop {
            let Link::Mqtt(decoder) = &mut s.link else {
                return changed;
            };
            let Some(packet) = decoder.next_packet() else {
                return changed;
            };
            match packet {
                Incoming::ConnAck(0) => {
                    log_info!("victron-solar: MQTT session up, looking for the portal id");
                    socket.write(&mqtt::subscribe(1, &[venus::DISCOVERY_FILTER]));
                }
                Incoming::ConnAck(code) => {
                    log_warn!("victron-solar: MQTT connection refused (code {})", code);
                    s.status = Some(Status::Refused(code));
                }
                Incoming::Publish { topic, payload } => match s.portal.clone() {
                    Some(portal) => changed |= s.readings.apply(&portal, &topic, &payload),
                    None => {
                        if let Some(portal) = venus::portal_from_serial_topic(&topic) {
                            let filters = venus::subscriptions(portal);
                            let filters: Vec<&str> = filters.iter().map(String::as_str).collect();
                            socket.write(&mqtt::subscribe(2, &filters));
                            socket.write(&mqtt::publish(&fmt!("R/{}/keepalive", portal), b""));
                            s.last_keepalive_secs = now_secs();
                            log_info!("victron-solar: portal id {}, subscribed", portal);
                            adopt_portal(s, portal);
                        }
                    }
                },
                Incoming::Other => {}
            }
        }
    }

    fn on_modbus_data(s: &mut Session, socket: &Socket, bytes: &[u8]) -> bool {
        let Link::Modbus(decoder, poller) = &mut s.link else {
            return false;
        };
        decoder.push(bytes);
        let mut changed = false;
        let mut portal = None;
        while let Some(response) = decoder.next_response() {
            let step = poller.on_response(response, &mut s.readings);
            if let Some(next) = &step.send {
                socket.write(next);
            }
            changed |= step.changed;
            match step.event {
                Some(modbus::Event::Portal(serial)) => {
                    log_info!(
                        "victron-solar: serial {}, scanning for solar devices",
                        serial
                    );
                    poller.yield_counters = store_get(&yield_counters_key(&serial))
                        .and_then(|t| YieldCounters::from_text(&t))
                        .unwrap_or_default();
                    portal = Some(serial);
                }
                Some(modbus::Event::Found {
                    chargers,
                    pv_inverters,
                }) => {
                    log_info!(
                        "victron-solar: found {} solar charger(s), {} PV inverter(s)",
                        chargers,
                        pv_inverters
                    );
                    if chargers + pv_inverters == 0 {
                        s.status = Some(Status::NoSolar);
                    }
                }
                None => {}
            }
            if poller.yield_counters.dirty
                && let Some(p) = s.portal.as_deref()
            {
                poller.yield_counters.dirty = false;
                store_put(&yield_counters_key(p), &poller.yield_counters.to_text());
            }
        }
        if let Some(portal) = portal {
            adopt_portal(s, &portal);
        }
        changed
    }

    /// Re-derive what's on screen from the readings, rolling the ledger
    /// over (and persisting it) when the day changed.
    fn recompute() {
        let (today, hour) = local_now();
        let (power, totals) = SESSION.with_borrow_mut(|s| {
            let power = s.readings.pv_power_w();
            let lifetime = s.readings.lifetime_kwh();
            let totals = match (s.readings.complete, lifetime, s.portal.as_deref()) {
                (true, Some(lifetime), Some(portal)) => {
                    let daily = s.readings.daily_history_kwh();
                    let changed = if let Some(ledger) = s.ledger.as_mut() {
                        ledger.observe(today, hour, lifetime, &daily)
                    } else {
                        s.ledger = Some(Ledger::seed(today, lifetime, &daily));
                        log_info!(
                            "victron-solar: started the totals from {} day(s) of history",
                            daily.len()
                        );
                        true
                    };
                    let ledger = s.ledger.as_ref().expect("set above");
                    if changed {
                        store_put(&ledger_key(portal), &ledger.to_text());
                        // Daily, so a rollover while the widget is off is still caught.
                        if let Link::Modbus(_, poller) = &s.link {
                            let text = poller.yield_counters.to_text();
                            store_put(&yield_counters_key(portal), &text);
                        }
                    }
                    Some(ledger.totals(lifetime))
                }
                _ => None,
            };
            (power, totals)
        });
        DATA.with_borrow_mut(|d| {
            d.pv_power_w = power.into();
            apply_totals(d, totals);
        });
        request_frame();
    }

    fn apply_totals(d: &mut SolarData, totals: Option<Totals>) {
        let Some(t) = totals else {
            return;
        };
        d.today_kwh = Availability::Available(t.today);
        d.month_kwh = Availability::Available(t.month);
        d.month_since = t.month_since;
        d.last_month_kwh = t.last_month.into();
        d.year_kwh = Availability::Available(t.year);
        d.year_since = t.year_since;
        d.last_year_kwh = t.last_year.into();
    }

    fn on_tick(_timer: &Timer, _periods: u32) {
        let now = now_secs();
        let stale = SESSION.with_borrow(|s| now - s.last_rx_secs > STALE_SECS);
        if SOCKET.with_borrow(Option::is_none) || stale {
            if stale {
                log_info!("victron-solar: no data from the device, reconnecting");
            }
            connect();
            return;
        }
        let out: Vec<Vec<u8>> = SESSION.with_borrow_mut(|s| match &mut s.link {
            Link::Mqtt(_) => {
                if now - s.last_keepalive_secs < KEEPALIVE_SECS {
                    return Vec::new();
                }
                s.last_keepalive_secs = now;
                let mut out = vec![mqtt::pingreq()];
                if let Some(portal) = &s.portal {
                    out.push(mqtt::publish(
                        &fmt!("R/{}/keepalive", portal),
                        KEEPALIVE_QUIET,
                    ));
                }
                out
            }
            Link::Modbus(_, poller) => poller.request_round(&mut s.readings).into_iter().collect(),
        });
        SOCKET.with_borrow(|socket| {
            if let Some(socket) = socket {
                for bytes in &out {
                    socket.write(bytes);
                }
            }
        });
        // Rolls the day over even when the sun is down and nothing changes.
        recompute();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        TICK.set(timer::start(TICK_MS, on_tick));
        if TICK.with_borrow(Option::is_none) {
            log_warn!("victron-solar: host refused the poll timer");
        }
        connect();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        log_info!("victron-solar: settings changed, reconnecting");
        // Maybe a different device: its totals are picked up by its own id.
        DATA.set(SolarData::default());
        SESSION.with_borrow_mut(|s| s.ledger = None);
        connect();
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    #[derive(Clone, Copy)]
    struct SizeParams {
        header_font: u32,
        icon_px: f32,
        power_font: u32,
        label_font: u32,
        value_font: u32,
        sub_font: u32,
        padding: f32,
        gap: f32,
        show_sub: bool,
    }

    const SMALL: SizeParams = SizeParams {
        header_font: 16,
        icon_px: 18.0,
        power_font: 72,
        label_font: 13,
        value_font: 18,
        sub_font: 11,
        padding: 12.0,
        gap: 6.0,
        show_sub: false,
    };
    const MEDIUM: SizeParams = SizeParams {
        header_font: 18,
        icon_px: 20.0,
        power_font: 101,
        label_font: 14,
        value_font: 22,
        sub_font: 12,
        padding: 14.0,
        gap: 8.0,
        show_sub: true,
    };
    const LARGE: SizeParams = SizeParams {
        header_font: 20,
        icon_px: 22.0,
        power_font: 130,
        label_font: 16,
        value_font: 26,
        sub_font: 13,
        padding: 16.0,
        gap: 10.0,
        show_sub: true,
    };
    const FULL: SizeParams = SizeParams {
        header_font: 24,
        icon_px: 28.0,
        power_font: 173,
        label_font: 20,
        value_font: 34,
        sub_font: 16,
        padding: 24.0,
        gap: 14.0,
        show_sub: true,
    };

    fn size_params(variant: SizeVariant) -> &'static SizeParams {
        match variant {
            SizeVariant::Small => &SMALL,
            SizeVariant::Medium => &MEDIUM,
            SizeVariant::Large => &LARGE,
            SizeVariant::Full => &FULL,
        }
    }

    fn header(font: u32, icon_px: f32) -> Node {
        row(
            props!(gap: 8.0, cross_align: CrossAlign::Center),
            [
                canvas(
                    props!(width: icon_px, height: icon_px),
                    [Draw::svg(0.0, 0.0, icon_px, icon_px, &SUN_ICON, SUN_COLOR)],
                ),
                text(
                    "Solar",
                    style!(size: font, weight: FontWeight::BOLD, color: HEADER_COLOR),
                ),
            ],
        )
    }

    fn power_str(data: &SolarData) -> String {
        match data.pv_power_w {
            Availability::Available(w) => fmt!("{} W", format_number!(w, 0)),
            Availability::Unavailable | Availability::Failed => NOT_AVAILABLE.to_string(),
        }
    }

    /// Width of `text` in DeckSans Bold at 1 px, from the font's advances
    /// (its digits are tabular); the SDK can't measure text.
    fn bold_width_em(text: &str) -> f32 {
        text.chars()
            .map(|c| match c {
                ',' | '.' => 0.310,
                ' ' | '\u{a0}' => 0.236,
                'W' => 0.973,
                '-' => 0.403,
                _ => 0.600,
            })
            .sum()
    }

    /// The live power figure at `size`, shrunk only as far as it must be to
    /// fit between the paddings of a `width` wide widget: the small layout
    /// has room for four digits, not five.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "widget widths and font sizes are small positive pixel counts"
    )]
    fn power_text(data: &SolarData, size: u32, width: u32, padding: f32) -> Node {
        let power = power_str(data);
        let room = width as f32 - 2.0 * padding;
        let fit = (room / bold_width_em(&power)).floor().max(1.0) as u32;
        text(
            power,
            style!(size: size.min(fit), weight: FontWeight::BOLD, color: WHITE, family: FontFamily::DeckSans, line_height: 0.9),
        )
    }

    fn kwh_str(v: Availability<f64>) -> String {
        match v {
            Availability::Available(v) => fmt!("{} kWh", format_number!(v, 1)),
            Availability::Unavailable | Availability::Failed => NOT_AVAILABLE.to_string(),
        }
    }

    fn stat_row(sp: &SizeParams, label: &str, value: Availability<f64>) -> Node {
        row(
            props!(cross_align: CrossAlign::Center),
            [
                text(
                    label,
                    style!(size: sp.label_font, color: GRAY_50, flex: 1.0),
                ),
                text(
                    kwh_str(value),
                    style!(size: sp.value_font, weight: FontWeight::BOLD, color: WHITE, align: TextAlign::Right),
                ),
            ],
        )
    }

    /// `sub`, when shown, is the line under a running total: the previous
    /// completed cycle's total (e.g. last month's yield), or, while the
    /// running total doesn't cover the whole cycle yet, where it starts.
    fn stat_block(
        sp: &SizeParams,
        label: &str,
        value: Availability<f64>,
        sub: Option<String>,
    ) -> Node {
        let mut children = vec![stat_row(sp, label, value)];
        if sp.show_sub
            && let Some(sub) = sub
        {
            children.push(text(sub, style!(size: sp.sub_font, color: GRAY_50)));
        }
        col(props!(gap: 2.0), children)
    }

    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    fn sub_line(since: Option<Date>, last_label: &str, last: Availability<f64>) -> String {
        since.map_or_else(
            || fmt!("{} {}", last_label, kwh_str(last)),
            |d| {
                fmt!(
                    "Counting since {} {}",
                    d.d,
                    MONTHS[(d.m as usize).clamp(1, 12) - 1]
                )
            },
        )
    }

    fn stats(sp: &SizeParams, data: &SolarData) -> Vec<Node> {
        vec![
            stat_block(sp, "Today", data.today_kwh, None),
            stat_block(
                sp,
                "This month",
                data.month_kwh,
                Some(sub_line(
                    data.month_since,
                    "Last month:",
                    data.last_month_kwh,
                )),
            ),
            stat_block(
                sp,
                "This year",
                data.year_kwh,
                Some(sub_line(data.year_since, "Last year:", data.last_year_kwh)),
            ),
        ]
    }

    /// Shown under the header until live data flows.
    fn status_line(data: &SolarData) -> Option<String> {
        match data.status {
            Status::Live => None,
            Status::Connecting => Some("Connecting…".to_string()),
            Status::Unreachable => Some(fmt!(
                "Not reachable — check the address and that {} is enabled",
                data.service
            )),
            Status::Refused(code) => {
                Some(fmt!("MQTT login refused (code {}) — password set?", code))
            }
            Status::NoSolar => Some("No solar charger or PV inverter found".to_string()),
        }
    }

    fn status_node(sp: &SizeParams, data: &SolarData) -> Option<Node> {
        status_line(data).map(|s| text(s, style!(size: sp.sub_font, color: ORANGE_50)))
    }

    fn view_rect(ws: WidgetSize, sp: &SizeParams, data: &SolarData) -> Node {
        let mut top = vec![header(sp.header_font, sp.icon_px)];
        top.extend(status_node(sp, data));
        col(
            props!(background: BG_COLOR, padding: sp.padding, gap: sp.gap),
            [
                col(props!(gap: 2.0), top),
                power_text(data, sp.power_font, ws.width, sp.padding),
                col(
                    props!(gap: sp.gap, flex: 1.0, justify_content: Justify::End),
                    stats(sp, data),
                ),
            ],
        )
    }

    /// Round layout (BFM100): everything centered and scaled off the
    /// viewport's diameter, since the rectangular layout's edge-pinned
    /// header would spill past the circular cutout.
    fn view_round(ws: WidgetSize, data: &SolarData) -> Node {
        let scale = ws.round_scale();
        let sp = SizeParams {
            header_font: scale_font(20, scale),
            icon_px: 22.0 * scale,
            power_font: scale_font(115, scale),
            label_font: scale_font(15, scale),
            value_font: scale_font(22, scale),
            sub_font: scale_font(13, scale),
            padding: 48.0 * scale,
            gap: 8.0 * scale,
            show_sub: true,
        };
        let mut children = vec![header(sp.header_font, sp.icon_px)];
        children.extend(status_node(&sp, data));
        children.push(power_text(data, sp.power_font, ws.width, sp.padding));
        children.extend(stats(&sp, data));
        col(
            props!(background: BG_COLOR, padding: sp.padding, gap: sp.gap, cross_align: CrossAlign::Center),
            children,
        )
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let shape = widget_viewport().shape;
        let data = DATA.with(|d| *d.borrow());
        let root = if matches!(shape, ViewportShape::Round) {
            view_round(ws, &data)
        } else {
            view_rect(ws, size_params(ws.variant), &data)
        };
        let _ = render_ui(ws.width, ws.height, root);
    }
}
