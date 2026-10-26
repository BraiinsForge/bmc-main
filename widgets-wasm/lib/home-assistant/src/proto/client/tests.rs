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

use serde_json::{Value, json};

use bmc_wasm_sdk::wasm_test;

use super::*;
use crate::proto::json::test_support::Message;
use crate::proto::{AttributeValue, Presence, Reply, Values};

const URL: &str = "ws://ha.test/api/websocket";
const TOKEN: &str = "token-as-the-transport-takes-it";

/// Stands in for the transport's placeholder syntax.
fn reserved(text: &str) -> bool {
    text.contains("RESERVED")
}

fn client() -> Session {
    Session::new(Setup {
        websocket_url: URL.to_owned(),
        token: TOKEN.to_owned(),
        reserved,
    })
}

fn feed(client: &mut Session, message: &Value) -> Output {
    client.on_message(&Message::parse(&message.to_string()))
}

/// What a test reads the commands from, whichever kind of call returned them.
trait Commands {
    fn commands(&self) -> &[Command];
}

impl Commands for Effects {
    fn commands(&self) -> &[Command] {
        &self.commands
    }
}

impl Commands for Output {
    fn commands(&self) -> &[Command] {
        &self.effects.commands
    }
}

/// The JSON of every message the output sends.
fn sent(out: &impl Commands) -> Vec<Value> {
    out.commands()
        .iter()
        .filter_map(|command| match command {
            Command::Send(text) => {
                Some(serde_json::from_str(text).expect("BUG: the core sends valid JSON"))
            }
            Command::Connect { .. } | Command::Close => None,
        })
        .collect()
}

fn connects(out: &impl Commands) -> bool {
    out.commands().contains(&Command::Connect {
        url: URL.to_owned(),
    })
}

fn closes(out: &impl Commands) -> bool {
    out.commands().contains(&Command::Close)
}

/// A client past `auth_ok`, with everything it sent on the way discarded.
fn ready(client: &mut Session) -> Output {
    let _ = client.start();
    let _ = feed(
        client,
        &json!({"type": "auth_required", "ha_version": "2026.10.0"}),
    );
    feed(
        client,
        &json!({"type": "auth_ok", "ha_version": "2026.10.0"}),
    )
}

fn subscribe(
    client: &mut Session,
    entities: &[&str],
    attributes: &[&str],
) -> (SubscriptionId, Effects) {
    client
        .subscribe(entities, attributes)
        .expect("BUG: the test subscription is within the bounds")
}

fn press(client: &mut Session) -> (CallId, Effects) {
    client
        .call_service("button", "press", "button.gate", &ServiceData::new())
        .expect("BUG: the test call is valid while connected")
}

/// A ready client whose connection answered a ping,
/// so a drop is retried at once, with everything it sent on the way discarded.
fn proven(client: &mut Session) -> Output {
    let _ = ready(client);
    let ping = sent(&client.on_timer());
    let [ping] = ping.as_slice() else {
        panic!("BUG: a ready client's timer sends one ping, not {ping:?}");
    };
    feed(client, &json!({"id": ping["id"], "type": "pong"}))
}

/// Drop a proven client's socket and fail the immediate reconnect, leaving it in backoff.
fn drop_and_fail_reconnect(client: &mut Session) -> Output {
    let reconnect = client.on_close(Closed::Dropped);
    assert!(
        connects(&reconnect),
        "a dropped connection is retried at once"
    );
    client.on_close(Closed::Dropped)
}

#[wasm_test]
fn starting_connects_to_the_slots_websocket_address() {
    let mut client = client();
    let out = client.start();

    assert!(connects(&out));
    assert_eq!(
        out.wake,
        Some(Wake::In(10)),
        "the connect has 10 s until auth_ok"
    );
    assert_eq!(client.status(), Status::Connecting);
}

#[wasm_test]
fn login_sends_the_token_and_nothing_else() {
    let mut client = client();
    let _ = client.start();
    let out = feed(&mut client, &json!({"type": "auth_required"}));

    assert_eq!(sent(&out), [json!({"type": "auth", "access_token": TOKEN})]);
}

#[wasm_test]
fn auth_ok_makes_the_client_ready_and_schedules_the_first_ping() {
    let mut client = client();
    let out = ready(&mut client);

    assert_eq!(client.status(), Status::Ready);
    assert_eq!(out.events, [Event::StatusChanged(Status::Ready)]);
    assert_eq!(out.effects.wake, Some(Wake::In(30)));
}

#[wasm_test]
fn a_rejected_token_stops_the_client_until_resumed() {
    let mut client = client();
    let _ = client.start();
    let _ = feed(&mut client, &json!({"type": "auth_required"}));
    let out = feed(
        &mut client,
        &json!({"type": "auth_invalid", "message": "Invalid access token"}),
    );

    assert!(closes(&out));
    assert_eq!(client.status(), Status::TokenRejected);
    assert_eq!(out.effects.wake, Some(Wake::Never));
    assert!(
        client.on_timer().effects.commands.is_empty(),
        "a stopped client does not retry"
    );

    let resumed = client.resume();
    assert!(connects(&resumed));
    assert_eq!(client.status(), Status::Connecting);
}

#[wasm_test]
fn subscriptions_made_before_login_go_out_after_it_in_order() {
    let mut client = client();
    let (first, _) = subscribe(&mut client, &["button.gate"], &["friendly_name"]);
    let (second, _) = subscribe(&mut client, &["sensor.outdoor"], &[]);
    assert_ne!(first, second);

    let out = ready(&mut client);
    assert_eq!(
        sent(&out),
        [
            json!({"id": 1, "type": "subscribe_entities", "entity_ids": ["button.gate"]}),
            json!({"id": 2, "type": "subscribe_entities", "entity_ids": ["sensor.outdoor"]}),
        ]
    );
}

#[wasm_test]
fn a_subscription_made_while_connected_goes_out_at_once() {
    let mut client = client();
    let _ = ready(&mut client);
    let (_, out) = subscribe(&mut client, &["sensor.a", "sensor.b", "sensor.a"], &[]);

    assert_eq!(
        sent(&out),
        [json!({"id": 1, "type": "subscribe_entities", "entity_ids": ["sensor.a", "sensor.b"]})],
        "a repeated entity is subscribed once"
    );
}

#[wasm_test]
fn the_snapshot_reports_present_and_missing_entities() {
    let mut client = client();
    let (sub, _) = subscribe(
        &mut client,
        &["button.gate", "button.ghost"],
        &["friendly_name"],
    );
    let _ = ready(&mut client);
    let _ = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": true, "result": null}),
    );

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"a": {
            "button.gate": {"s": "unknown", "a": {"friendly_name": "Gate"}, "c": "01HX", "lc": 1.0}
        }}}),
    );

    let gate = client.entity(sub, "button.gate").expect("BUG: subscribed");
    assert_eq!(gate.presence, Presence::Present);
    assert_eq!(gate.state.as_deref(), Some("unknown"));
    assert_eq!(gate.text("friendly_name"), Some("Gate"));
    assert_eq!(
        client
            .entity(sub, "button.ghost")
            .map(|entity| entity.presence),
        Some(Presence::Missing)
    );
    assert_eq!(
        out.events.len(),
        2,
        "both entities changed: {:?}",
        out.events
    );
}

#[wasm_test]
fn events_reach_only_the_subscription_whose_request_they_answer() {
    let mut client = client();
    let (buttons, _) = subscribe(&mut client, &["button.gate"], &[]);
    let (sensors, _) = subscribe(&mut client, &["sensor.outdoor"], &[]);
    let _ = ready(&mut client);

    let out = feed(
        &mut client,
        &json!({"id": 2, "type": "event", "event": {"a": {"sensor.outdoor": {"s": "21.5", "a": {}}}}}),
    );

    assert_eq!(
        out.events,
        [Event::EntityChanged {
            subscription: sensors,
            entity_id: "sensor.outdoor".to_owned()
        }]
    );
    assert_eq!(
        client
            .entity(buttons, "button.gate")
            .map(|entity| entity.presence),
        Some(Presence::Awaiting),
        "the other subscription's snapshot is still to come"
    );
}

#[wasm_test]
fn a_subscriptions_events_are_unwanted_once_it_is_unsubscribed() {
    let mut client = client();
    let (subscription, _) = subscribe(&mut client, &["sensor.outdoor"], &[]);
    let changed = Event::EntityChanged {
        subscription,
        entity_id: "sensor.outdoor".to_owned(),
    };
    assert!(client.wants(&changed));

    let _ = client.unsubscribe(subscription);

    assert!(!client.wants(&changed));
    assert!(
        client.wants(&Event::StatusChanged(Status::Ready)),
        "the connection's own events always reach the widget"
    );
}

#[wasm_test]
fn a_change_after_the_snapshot_updates_the_entity() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["script.open_gate"], &[]);
    let _ = ready(&mut client);
    let _ = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"a": {"script.open_gate": {"s": "off", "a": {}}}}}),
    );

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"c": {"script.open_gate": {"+": {"s": "on"}}}}}),
    );

    assert_eq!(out.events.len(), 1);
    assert_eq!(
        client
            .entity(sub, "script.open_gate")
            .and_then(|entity| entity.state.as_deref()),
        Some("on")
    );
}

#[wasm_test]
fn an_event_naming_none_of_a_subscriptions_entities_leaves_them_alone() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["script.open_gate", "sensor.outdoor"], &[]);
    let _ = ready(&mut client);
    let _ = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"a": {
            "script.open_gate": {"s": "off", "a": {}},
            "sensor.outdoor": {"s": "21.5", "a": {}},
        }}}),
    );
    let before = client.entity(sub, "sensor.outdoor").cloned();

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"c": {"sensor.other": {"+": {"s": "1"}}}}}),
    );

    assert!(out.events.is_empty(), "{:?}", out.events);
    assert_eq!(client.entity(sub, "sensor.outdoor").cloned(), before);
}

#[wasm_test]
fn an_entity_the_event_removes_goes_missing() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["script.open_gate", "sensor.outdoor"], &[]);
    let _ = ready(&mut client);
    let _ = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"a": {
            "script.open_gate": {"s": "off", "a": {}},
            "sensor.outdoor": {"s": "21.5", "a": {}},
        }}}),
    );

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"r": ["sensor.outdoor"]}}),
    );

    assert_eq!(
        out.events,
        [Event::EntityChanged {
            subscription: sub,
            entity_id: "sensor.outdoor".to_owned()
        }]
    );
    assert_eq!(
        client
            .entity(sub, "sensor.outdoor")
            .map(|entity| entity.presence),
        Some(Presence::Missing)
    );
}

#[wasm_test]
fn a_service_call_is_sent_with_its_target_and_its_result_reported() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, out) = press(&mut client);

    assert_eq!(
        sent(&out),
        [json!({
            "id": 1, "type": "call_service", "domain": "button", "service": "press",
            "target": {"entity_id": "button.gate"},
        })]
    );

    let done = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": true, "result": {"context": {}}}),
    );
    assert_eq!(
        done.events,
        [Event::CallFinished {
            call,
            result: Ok(Reply::Done)
        }]
    );
}

#[wasm_test]
fn service_data_is_sent_beside_the_target() {
    let mut client = client();
    let _ = ready(&mut client);
    let data = ServiceData::new()
        .integer("brightness", 128)
        .boolean("flash", false)
        .text("effect", "rainbow");
    let (_, out) = client
        .call_service("light", "turn_on", "light.desk", &data)
        .expect("BUG: a valid call");

    assert_eq!(
        sent(&out)[0]["service_data"],
        json!({"brightness": 128, "flash": false, "effect": "rainbow"})
    );
}

#[wasm_test]
fn a_decimal_is_sent_as_a_json_number() {
    let mut client = client();
    let _ = ready(&mut client);
    let data = ServiceData::new().decimal("temperature", 21.5, 1);
    let (_, out) = client
        .call_service("climate", "set_temperature", "climate.living_room", &data)
        .expect("BUG: a valid call");

    assert_eq!(sent(&out)[0]["service_data"], json!({"temperature": 21.5}));
}

#[wasm_test]
fn a_decimal_json_cannot_spell_is_refused_before_sending() {
    let mut client = client();
    let _ = ready(&mut client);

    for value in [f64::NAN, f64::INFINITY, 1e300] {
        let data = ServiceData::new().decimal("temperature", value, 1);
        assert_eq!(
            client
                .call_service("climate", "set_temperature", "climate.living_room", &data)
                .err(),
            Some(RequestError::InvalidData),
            "{value}"
        );
    }
}

#[wasm_test]
fn lists_are_sent_as_json_arrays() {
    let mut client = client();
    let _ = ready(&mut client);
    let data = ServiceData::new()
        .integers("rgb_color", &[255, 128, 0])
        .decimals("hs_color", &[30.04, 100.0], 1)
        .texts("entity_ids", &["light.a", "light.b"]);
    let (_, out) = client
        .call_service("light", "turn_on", "light.desk", &data)
        .expect("BUG: a valid call");

    assert_eq!(
        sent(&out)[0]["service_data"],
        json!({"rgb_color": [255, 128, 0], "hs_color": [30.0, 100.0], "entity_ids": ["light.a", "light.b"]})
    );
}

#[wasm_test]
fn a_list_past_its_bounds_is_refused_before_sending() {
    let mut client = client();
    let _ = ready(&mut client);
    let long_text = "x".repeat(MAX_SERVICE_DATA_TEXT_BYTES + 1);
    let cases = [
        ServiceData::new().decimals("hs_color", &[30.0, f64::NAN], 1),
        ServiceData::new().texts("entity_ids", &[&long_text]),
        ServiceData::new().integers("rgb_color", &[0; MAX_SERVICE_DATA_LIST_ITEMS + 1]),
    ];

    for data in cases {
        assert_eq!(
            client
                .call_service("light", "turn_on", "light.desk", &data)
                .err(),
            Some(RequestError::InvalidData),
            "{data:?}"
        );
    }
}

#[wasm_test]
fn a_data_key_is_any_text_within_the_name_cap() {
    let mut client = client();
    let _ = ready(&mut client);
    let call = |client: &mut Session, key: &str| {
        let data = ServiceData::new().integer(key, 1);
        client
            .call_service("light", "turn_on", "light.desk", &data)
            .err()
    };

    assert_eq!(call(&mut client, "venueId"), None);
    let too_long = "k".repeat(MAX_NAME_BYTES + 1);
    for key in ["", too_long.as_str()] {
        assert_eq!(
            call(&mut client, key),
            Some(RequestError::InvalidName),
            "the key is the problem, not its value"
        );
    }
}

#[wasm_test]
fn text_the_transport_reserves_is_refused_and_only_that_request_fails() {
    let mut client = client();
    let _ = ready(&mut client);
    let reserved_data = [
        ServiceData::new().text("message", "a RESERVED text"),
        ServiceData::new().integer("RESERVED", 1),
        ServiceData::new().texts("entity_id", &["light.a", "RESERVED"]),
    ];

    for data in &reserved_data {
        assert_eq!(
            client
                .call_service("notify", "send_message", "notify.phone", data)
                .err(),
            Some(RequestError::InvalidData),
            "{data:?}"
        );
    }
    assert_eq!(
        client
            .command("RESERVED/get", &ServiceData::new(), &[])
            .err(),
        Some(RequestError::InvalidData)
    );
    assert_eq!(client.status(), Status::Ready);
    assert!(
        client
            .call_service(
                "notify",
                "send_message",
                "notify.phone",
                &ServiceData::new().text("message", "{{ user }} braces pass"),
            )
            .is_ok()
    );
}

#[wasm_test]
fn a_text_past_its_bounds_is_refused_as_data() {
    let mut client = client();
    let _ = ready(&mut client);
    let send = |client: &mut Session, len: usize| {
        let data = ServiceData::new().text("value", &"x".repeat(len));
        client
            .call_service("input_text", "set_value", "input_text.note", &data)
            .err()
    };

    assert_eq!(
        send(&mut client, MAX_SERVICE_DATA_TEXT_BYTES),
        None,
        "a text at the cap goes out"
    );
    assert_eq!(
        send(&mut client, MAX_SERVICE_DATA_TEXT_BYTES + 1),
        Some(RequestError::InvalidData)
    );
}

#[wasm_test]
fn a_rejected_call_reports_home_assistants_code_and_message() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, _) = press(&mut client);

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": false,
               "error": {"code": "not_found", "message": "Service not found."}}),
    );

    assert_eq!(
        out.events,
        [Event::CallFinished {
            call,
            result: Err(CallFailure::Rejected(HaError {
                code: "not_found".to_owned(),
                message: "Service not found.".to_owned(),
            })),
        }]
    );
}

#[wasm_test]
fn a_failure_naming_no_error_reports_the_unknown_error() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, _) = press(&mut client);

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": false}),
    );

    assert_eq!(
        out.events,
        [Event::CallFinished {
            call,
            result: Err(CallFailure::Rejected(HaError {
                code: UNKNOWN_ERROR.to_owned(),
                message: String::new(),
            })),
        }]
    );
}

#[wasm_test]
fn a_call_is_refused_while_disconnected_rather_than_queued() {
    let mut client = client();
    let _ = client.start();

    assert_eq!(
        client
            .call_service("button", "press", "button.gate", &ServiceData::new())
            .err(),
        Some(RequestError::NotConnected),
        "a press must not run later, when nobody is looking"
    );
}

#[wasm_test]
fn pending_calls_are_bounded() {
    let mut client = client();
    let _ = ready(&mut client);
    for _ in 0..MAX_PENDING_CALLS {
        let _ = press(&mut client);
    }

    assert_eq!(
        client
            .call_service("button", "press", "button.gate", &ServiceData::new())
            .err(),
        Some(RequestError::TooManyPendingCalls)
    );
}

#[wasm_test]
fn a_dropped_connection_fails_its_calls_and_never_resends_them() {
    let mut client = client();
    let _ = proven(&mut client);
    let (call, _) = press(&mut client);

    let dropped = client.on_close(Closed::Dropped);
    assert!(
        dropped.events.contains(&Event::CallFinished {
            call,
            result: Err(CallFailure::Disconnected)
        }),
        "{:?}",
        dropped.events
    );

    let _ = feed(&mut client, &json!({"type": "auth_required"}));
    let relogin = feed(&mut client, &json!({"type": "auth_ok"}));
    assert!(
        sent(&relogin)
            .iter()
            .all(|message| message["type"] != "call_service"),
        "a press Home Assistant may already have run must not run twice"
    );
}

#[wasm_test]
fn a_late_answer_to_a_failed_call_is_ignored() {
    let mut client = client();
    let _ = proven(&mut client);
    let (_, pressed) = press(&mut client);
    let id = sent(&pressed)[0]["id"].clone();
    assert!(connects(&client.on_close(Closed::Dropped)));
    let _ = feed(&mut client, &json!({"type": "auth_required"}));
    let _ = feed(&mut client, &json!({"type": "auth_ok"}));

    let late = feed(
        &mut client,
        &json!({"id": id, "type": "result", "success": true}),
    );
    assert!(late.events.is_empty());
}

const PRECISION: &str = "/options/sensor/display_precision";
const SUGGESTED: &str = "/options/sensor/suggested_display_precision";

fn look_up_registry_entry(client: &mut Session) -> (CallId, Effects) {
    client
        .command(
            "config/entity_registry/get",
            &ServiceData::new().text("entity_id", "sensor.outdoor"),
            &[PRECISION, SUGGESTED],
        )
        .expect("BUG: the test command is valid while connected")
}

#[wasm_test]
fn a_command_is_sent_with_its_fields_beside_its_id_and_type() {
    let mut client = client();
    let _ = ready(&mut client);
    let (_, out) = look_up_registry_entry(&mut client);

    assert_eq!(
        sent(&out),
        [json!({"id": 1, "type": "config/entity_registry/get", "entity_id": "sensor.outdoor"})]
    );
}

#[wasm_test]
fn a_command_reply_holds_the_wanted_values_its_result_has() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, _) = look_up_registry_entry(&mut client);

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": true, "result": {
            "entity_id": "sensor.outdoor",
            "options": {"sensor": {"suggested_display_precision": 1}},
        }}),
    );

    let [
        Event::CallFinished {
            call: finished,
            result: Ok(Reply::Values(reply)),
        },
    ] = out.events.as_slice()
    else {
        panic!("BUG: expected one finished call, got {:?}", out.events);
    };
    assert_eq!(*finished, call);
    assert_eq!(reply.number(SUGGESTED), Some(1.0));
    assert_eq!(
        reply.get(PRECISION),
        None,
        "a value the result lacks is absent"
    );
    assert_eq!(
        reply.get("/entity_id"),
        None,
        "a value not asked for is not kept"
    );
}

#[wasm_test]
fn a_command_reply_reads_text_and_lists_too() {
    const UNIT: &str = "/unit_system/temperature";
    const COMPONENTS: &str = "/components";

    let mut client = client();
    let _ = ready(&mut client);
    let _ = client
        .command("get_config", &ServiceData::new(), &[UNIT, COMPONENTS])
        .expect("BUG: the test command is valid while connected");

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": true, "result": {
            "unit_system": {"temperature": "°C"},
            "components": ["climate", "demo"],
        }}),
    );

    let reply = only_values(&out);
    assert_eq!(reply.text(UNIT), Some("°C"));
    assert_eq!(reply.number(UNIT), None);
    assert_eq!(
        reply.get(COMPONENTS),
        Some(&AttributeValue::List(vec![
            "climate".to_owned(),
            "demo".to_owned()
        ]))
    );
}

#[wasm_test]
fn a_rejected_command_reports_home_assistants_code() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, _) = look_up_registry_entry(&mut client);

    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": false,
               "error": {"code": "not_found", "message": "Entity not found"}}),
    );

    assert_eq!(
        out.events,
        [Event::CallFinished {
            call,
            result: Err(CallFailure::Rejected(HaError {
                code: "not_found".to_owned(),
                message: "Entity not found".to_owned(),
            })),
        }]
    );
}

#[wasm_test]
fn a_command_shares_the_bound_on_pending_calls() {
    let mut client = client();
    let _ = ready(&mut client);
    for _ in 0..MAX_PENDING_CALLS {
        let _ = press(&mut client);
    }

    assert_eq!(
        client
            .command("config/entity_registry/get", &ServiceData::new(), &[])
            .err(),
        Some(RequestError::TooManyPendingCalls)
    );
}

#[wasm_test]
fn a_command_is_refused_while_disconnected() {
    let mut client = client();
    let _ = client.start();

    assert_eq!(
        client
            .command("config/entity_registry/get", &ServiceData::new(), &[])
            .err(),
        Some(RequestError::NotConnected)
    );
}

#[wasm_test]
fn the_sessions_own_commands_are_refused() {
    let mut client = client();
    let _ = ready(&mut client);
    for command in ["auth", "ping"] {
        assert_eq!(
            client.command(command, &ServiceData::new(), &[]).err(),
            Some(RequestError::InvalidName),
            "{command}: the session sends it itself",
        );
    }
}

#[wasm_test]
fn a_command_type_is_any_text_within_the_name_cap() {
    let mut client = client();
    let _ = ready(&mut client);

    for command in [
        "dynalite/get-config",
        "hacs/repository/info",
        "zwave_js/node_status",
    ] {
        assert!(
            client.command(command, &ServiceData::new(), &[]).is_ok(),
            "{command:?}"
        );
    }
}

#[wasm_test]
fn commands_are_checked() {
    let mut client = client();
    let _ = ready(&mut client);
    let empty = ServiceData::new();
    let too_many: Vec<String> = (0..=MAX_REPLY_VALUES)
        .map(|index| {
            let mut pointer = String::from("/v");
            pointer.push_str(&index.to_string());
            pointer
        })
        .collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let too_long = "c".repeat(MAX_NAME_BYTES + 1);

    let cases: [(&str, &ServiceData, &[&str], RequestError); 6] = [
        ("", &empty, &[], RequestError::InvalidName),
        (&too_long, &empty, &[], RequestError::InvalidName),
        (
            "get_config",
            &ServiceData::new().integer("id", 7),
            &[],
            RequestError::InvalidName,
        ),
        (
            "get_config",
            &ServiceData::new().text("type", "auth"),
            &[],
            RequestError::InvalidName,
        ),
        (
            "get_config",
            &empty,
            &too_many,
            RequestError::TooManyReplyValues,
        ),
        (
            "get_config",
            &empty,
            &["options"],
            RequestError::InvalidPointer,
        ),
    ];
    for (command, data, wanted, error) in cases {
        assert_eq!(
            client.command(command, data, wanted).err(),
            Some(error),
            "{command:?} {data:?} {wanted:?}"
        );
    }
}

fn only_reply(out: &Output) -> &Reply {
    let [
        Event::CallFinished {
            result: Ok(reply), ..
        },
    ] = out.events.as_slice()
    else {
        panic!("BUG: expected one finished call, got {:?}", out.events);
    };
    reply
}

fn only_values(out: &Output) -> &Values {
    let Reply::Values(values) = only_reply(out) else {
        panic!("BUG: expected values, got {:?}", out.events);
    };
    values
}

/// `count` distinct, well-formed entity ids.
fn entity_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let mut id = String::from("sensor.s");
            id.push_str(&i.to_string());
            id
        })
        .collect()
}

#[wasm_test]
fn a_subscription_counts_each_entity_once_against_the_limit() {
    let mut client = client();
    let mut ids = entity_ids(MAX_ENTITIES_PER_SUBSCRIPTION);
    ids.push(ids[0].clone());
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();

    assert!(client.subscribe(&ids, &[]).is_ok());
}

#[wasm_test]
fn reconnect_waits_one_more_second_per_failure_up_to_five() {
    let mut client = client();
    let _ = proven(&mut client);
    let mut out = drop_and_fail_reconnect(&mut client);

    for expected in [1, 2, 3, 4, 5, 5] {
        assert_eq!(out.effects.wake, Some(Wake::In(expected)));
        assert!(!connects(&out), "the attempt waits for the timer");
        let attempt = client.on_timer();
        assert!(connects(&attempt));
        out = client.on_close(Closed::Dropped);
    }
    assert_eq!(client.status(), Status::Disconnected);
}

/// Log in again after a failed attempt, leaving the connection ready but unproven.
fn log_in_after_the_backoff(client: &mut Session) {
    assert!(
        connects(&client.on_timer()),
        "the backoff ends in an attempt"
    );
    let _ = feed(client, &json!({"type": "auth_required"}));
    let _ = feed(client, &json!({"type": "auth_ok"}));
    assert_eq!(client.status(), Status::Ready);
}

#[wasm_test]
fn a_connection_that_answered_a_ping_resets_the_backoff() {
    let mut client = client();
    let _ = proven(&mut client);
    let _ = drop_and_fail_reconnect(&mut client);
    log_in_after_the_backoff(&mut client);
    let ping = client.on_timer();
    let _ = feed(
        &mut client,
        &json!({"id": sent(&ping)[0]["id"], "type": "pong"}),
    );

    let out = drop_and_fail_reconnect(&mut client);
    assert_eq!(out.effects.wake, Some(Wake::In(1)));
}

#[wasm_test]
fn a_connection_lost_before_its_first_pong_counts_as_a_failed_attempt() {
    let mut client = client();
    let _ = proven(&mut client);
    let _ = drop_and_fail_reconnect(&mut client);
    log_in_after_the_backoff(&mut client);

    let out = client.on_close(Closed::Dropped);
    assert!(
        !connects(&out),
        "a server dropping every socket after its login must not be redialled at once"
    );
    assert_eq!(out.effects.wake, Some(Wake::In(2)));
}

#[wasm_test]
fn a_first_connection_lost_right_after_its_login_backs_off() {
    let mut client = client();
    let _ = ready(&mut client);

    let out = client.on_close(Closed::Dropped);
    assert!(!connects(&out));
    assert_eq!(out.effects.wake, Some(Wake::In(1)));
}

#[wasm_test]
fn a_first_attempt_that_fails_reports_disconnected() {
    let mut client = client();
    let _ = client.start();
    let out = client.on_close(Closed::Dropped);

    assert_eq!(out.events, [Event::StatusChanged(Status::Disconnected)]);
    assert_eq!(out.effects.wake, Some(Wake::In(1)));
}

#[wasm_test]
fn a_connect_that_never_logs_in_times_out() {
    let mut client = client();
    let _ = client.start();
    let _ = feed(&mut client, &json!({"type": "auth_required"}));
    let out = client.on_timer();

    assert!(closes(&out));
    assert_eq!(out.effects.wake, Some(Wake::In(1)));
}

#[wasm_test]
fn each_tick_pings_and_arms_the_next_one_30_s_out() {
    let mut client = client();
    let _ = ready(&mut client);
    let out = client.on_timer();

    assert_eq!(sent(&out), [json!({"id": 1, "type": "ping"})]);
    assert_eq!(out.effects.wake, Some(Wake::In(30)));
}

#[wasm_test]
fn a_pong_leaves_the_tick_alone() {
    let mut client = client();
    let _ = ready(&mut client);
    let _ = client.on_timer();

    let pong = feed(&mut client, &json!({"id": 1, "type": "pong"}));
    assert_eq!(
        pong.effects.wake, None,
        "a slow pong must not push the next ping, and with it every call's timeout, later"
    );
    assert_eq!(
        sent(&client.on_timer()),
        [json!({"id": 2, "type": "ping"})],
        "the next tick pings again"
    );
}

#[wasm_test]
fn a_missing_pong_reconnects() {
    let mut client = client();
    let _ = proven(&mut client);
    let _ = client.on_timer();
    let out = client.on_timer();

    assert!(closes(&out));
    assert!(
        connects(&out),
        "a connection that was up is retried at once"
    );
    assert_eq!(client.status(), Status::Disconnected);
}

#[wasm_test]
fn a_pong_for_another_id_does_not_count() {
    let mut client = client();
    let _ = ready(&mut client);
    let _ = client.on_timer();
    let _ = feed(&mut client, &json!({"id": 99, "type": "pong"}));

    assert!(closes(&client.on_timer()));
}

#[wasm_test]
fn an_unanswered_call_times_out_at_the_second_tick() {
    let mut client = client();
    let _ = ready(&mut client);
    let (call, _) = press(&mut client);

    let first = client.on_timer();
    assert!(first.events.is_empty(), "one tick is not yet a timeout");
    let _ = feed(&mut client, &json!({"id": 2, "type": "pong"}));

    let second = client.on_timer();
    assert_eq!(
        second.events,
        [Event::CallFinished {
            call,
            result: Err(CallFailure::TimedOut)
        }]
    );
}

#[wasm_test]
fn a_refused_message_stops_as_not_configured() {
    let mut client = client();
    let _ = ready(&mut client);
    let out = client.on_close(Closed::CredentialRefused);

    assert_eq!(client.status(), Status::NotConfigured);
    assert_eq!(out.events, [Event::StatusChanged(Status::NotConfigured)]);
    assert_eq!(out.effects.wake, Some(Wake::Never));
    assert!(!connects(&out));
}

#[wasm_test]
fn a_host_close_the_same_request_would_repeat_stops_until_resumed() {
    for (closed, status) in [
        (Closed::MessageTooLarge, Status::MessageTooLarge),
        (Closed::UnsupportedScheme, Status::UnsupportedAddress),
        (Closed::InvalidRequest, Status::UnsupportedAddress),
    ] {
        let mut client = client();
        let _ = ready(&mut client);
        let out = client.on_close(closed);

        assert_eq!(client.status(), status);
        assert_eq!(out.events, [Event::StatusChanged(status)]);
        assert_eq!(out.effects.wake, Some(Wake::Never));
        assert!(
            !connects(&out),
            "{closed:?} must not reconnect into the same close"
        );
        assert!(connects(&client.resume()));
    }
}

#[wasm_test]
fn a_refused_connect_stops_as_not_configured_until_resumed() {
    let mut client = client();
    let _ = client.start();
    let out = client.on_close(Closed::CredentialRefused);

    assert_eq!(client.status(), Status::NotConfigured);
    assert_eq!(out.events, [Event::StatusChanged(Status::NotConfigured)]);
    assert_eq!(out.effects.wake, Some(Wake::Never));

    assert!(connects(&client.resume()));
    assert_eq!(
        client.status(),
        Status::Connecting,
        "the widget reads the status itself after resuming"
    );
}

#[wasm_test]
fn resuming_while_connecting_starts_over_on_a_new_socket() {
    let mut client = client();
    let _ = client.start();

    let resumed = client.resume();

    assert_eq!(
        resumed.commands,
        [
            Command::Close,
            Command::Connect {
                url: URL.to_owned()
            }
        ],
        "the old socket may carry credentials the widget has since replaced"
    );
    assert_eq!(client.status(), Status::Connecting);
}

#[wasm_test]
fn resuming_a_connected_client_does_nothing() {
    let mut client = client();
    let _ = ready(&mut client);

    assert_eq!(client.resume(), Effects::default());
    assert_eq!(client.status(), Status::Ready);
}

/// Past the widget's socket limit the host gives the transport no socket,
/// so nothing ever answers and the connect timeout retries it with backoff.
#[wasm_test]
fn a_connect_nothing_answers_is_retried_with_backoff() {
    let mut client = client();
    let _ = client.start();
    let out = client.on_timer();

    assert_eq!(client.status(), Status::Disconnected);
    assert!(!connects(&out), "another socket has to close first");
    assert_eq!(out.effects.wake, Some(Wake::In(1)));
}

#[wasm_test]
fn a_changed_account_reconnects_at_once_with_the_backoff_reset() {
    let mut client = client();
    let _ = client.start();
    let _ = client.on_close(Closed::Dropped);
    let _ = client.on_timer();
    let out = client.on_close(Closed::AccountChanged);

    assert!(
        connects(&out),
        "the new account deserves an attempt now, not after the backoff"
    );
}

#[wasm_test]
fn a_home_assistant_without_subscribe_entities_is_unsupported() {
    let mut client = client();
    let _ = subscribe(&mut client, &["button.gate"], &[]);
    let _ = ready(&mut client);
    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": false,
               "error": {"code": "unknown_command", "message": "Unknown command."}}),
    );

    assert!(closes(&out));
    assert_eq!(client.status(), Status::Unsupported);
}

#[wasm_test]
fn another_subscription_error_is_reported_and_the_client_stays_up() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["button.gate"], &[]);
    let _ = ready(&mut client);
    let out = feed(
        &mut client,
        &json!({"id": 1, "type": "result", "success": false,
               "error": {"code": "invalid_format", "message": "bad"}}),
    );

    assert_eq!(
        out.events,
        [Event::SubscriptionFailed {
            subscription: sub,
            error: HaError {
                code: "invalid_format".to_owned(),
                message: "bad".to_owned()
            },
        }]
    );
    assert_eq!(client.status(), Status::Ready);
}

#[wasm_test]
fn entities_go_stale_on_a_drop_and_fresh_on_the_next_snapshot() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["sensor.outdoor"], &[]);
    let _ = proven(&mut client);
    let snapshot = json!({"id": 1, "type": "event", "event": {"a": {"sensor.outdoor": {"s": "21.5", "a": {}}}}});
    let _ = feed(&mut client, &snapshot);

    let _ = client.on_close(Closed::Dropped);
    let stale = client
        .entity(sub, "sensor.outdoor")
        .expect("BUG: subscribed");
    assert_eq!(stale.presence, Presence::Stale);
    assert_eq!(
        stale.state.as_deref(),
        Some("21.5"),
        "the last value stays readable"
    );

    let _ = feed(&mut client, &json!({"type": "auth_required"}));
    let relogin = feed(&mut client, &json!({"type": "auth_ok"}));
    assert_eq!(
        sent(&relogin),
        [json!({"id": 1, "type": "subscribe_entities", "entity_ids": ["sensor.outdoor"]})],
        "the subscription is sent again, numbered afresh"
    );
    let _ = feed(&mut client, &snapshot);
    assert_eq!(
        client
            .entity(sub, "sensor.outdoor")
            .expect("BUG: subscribed")
            .presence,
        Presence::Present
    );
}

#[wasm_test]
fn unsubscribing_tells_home_assistant_and_ignores_later_events() {
    let mut client = client();
    let (sub, _) = subscribe(&mut client, &["sensor.outdoor"], &[]);
    let _ = ready(&mut client);
    let out = client.unsubscribe(sub);

    assert_eq!(
        sent(&out),
        [json!({"id": 2, "type": "unsubscribe_events", "subscription": 1})]
    );
    let late = feed(
        &mut client,
        &json!({"id": 1, "type": "event", "event": {"a": {"sensor.outdoor": {"s": "1", "a": {}}}}}),
    );
    assert!(late.events.is_empty());
    assert!(client.entity(sub, "sensor.outdoor").is_none());
}

#[wasm_test]
fn subscriptions_are_bounded_and_checked() {
    let mut client = client();
    let too_many: Vec<String> = (0..=MAX_ENTITIES_PER_SUBSCRIPTION)
        .map(|i| {
            let mut id = String::from("sensor.s");
            id.push(char::from(
                b'a' + u8::try_from(i % 26).expect("BUG: below 26"),
            ));
            id.push_str(&"x".repeat(i));
            id
        })
        .collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let too_many_attributes = attribute_names(MAX_ATTRIBUTES_PER_SUBSCRIPTION + 1);
    let too_many_attributes: Vec<&str> = too_many_attributes.iter().map(String::as_str).collect();

    for (entities, attributes, error) in [
        (&[][..], &[][..], RequestError::NoEntities),
        (&too_many[..], &[][..], RequestError::TooManyEntities),
        (
            &["sensor.ok"][..],
            &too_many_attributes[..],
            RequestError::TooManyAttributes,
        ),
        (
            &["Sensor.Upper"][..],
            &[][..],
            RequestError::InvalidEntityId,
        ),
        (&["sensor"][..], &[][..], RequestError::InvalidEntityId),
        (
            &["sensor._lead"][..],
            &[][..],
            RequestError::InvalidEntityId,
        ),
        (&["sensor.a__b"][..], &[][..], RequestError::InvalidEntityId),
        (&["sensor.ok"][..], &[""][..], RequestError::InvalidName),
    ] {
        assert_eq!(
            client.subscribe(entities, attributes).err(),
            Some(error),
            "{entities:?} {attributes:?}"
        );
    }

    for _ in 0..MAX_SUBSCRIPTIONS {
        let _ = subscribe(&mut client, &["sensor.ok"], &[]);
    }
    assert_eq!(
        client.subscribe(&["sensor.ok"], &[]).err(),
        Some(RequestError::TooManySubscriptions)
    );
}

/// `count` distinct attribute names.
fn attribute_names(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let mut name = String::from("attribute_");
            name.push_str(&i.to_string());
            name
        })
        .collect()
}

#[wasm_test]
fn a_subscription_counts_each_attribute_once_against_the_limit() {
    let mut client = client();
    let names = attribute_names(MAX_ATTRIBUTES_PER_SUBSCRIPTION);
    let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
    names.push(names[0]);

    assert!(client.subscribe(&["sensor.outdoor"], &names).is_ok());
}

#[wasm_test]
fn messages_outside_their_phase_are_ignored() {
    let mut client = client();
    let _ = client.start();

    for message in [
        json!({"type": "auth_ok"}),
        json!({"id": 1, "type": "result", "success": true}),
        json!({"id": 1, "type": "event", "event": {}}),
        json!({"id": 1, "type": "pong"}),
        json!({"no_type": true}),
    ] {
        let out = feed(&mut client, &message);
        assert_eq!(out, Output::default());
    }
    assert_eq!(client.status(), Status::Connecting);
}
