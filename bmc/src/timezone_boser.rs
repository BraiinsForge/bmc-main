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

use std::str::FromStr;
use std::sync::Arc;

use bmc_shared_time::time::Timezone;
use serde::Deserialize;
use tracing::{info, warn};

use crate::boser::{StateSink, StreamConfig};
use crate::manager::BmcManager;

const EVENTS_PATH: &str = "/api/v1/system/timezone/events";

#[derive(Debug, Deserialize)]
struct TimezoneResponse {
    id: String,
    label: String,
    offset: String,
}

pub(crate) fn spawn_observer<M: BmcManager>(
    config: StreamConfig,
    manager: Arc<M>,
) -> tokio::task::JoinHandle<()> {
    crate::boser::spawn(
        config,
        TimezoneSink {
            publisher: move |timezone| manager.publish_timezone(timezone),
        },
    )
}

struct TimezoneSink<P> {
    publisher: P,
}

impl<P> StateSink for TimezoneSink<P>
where
    P: Fn(Timezone) -> bool + Send + 'static,
{
    type State = TimezoneResponse;

    const PATH: &'static str = EVENTS_PATH;

    fn observe(&mut self, response: &Self::State) {
        match Timezone::from_str(&response.id) {
            Ok(timezone) => {
                let timezone_for_log = timezone.clone();
                if (self.publisher)(timezone) {
                    info!(timezone = %timezone_for_log, "Boser timezone updated");
                }
            }
            Err(error) => warn!(
                timezone_id = response.id,
                label = response.label,
                offset = response.offset,
                %error,
                "unrepresentable boser timezone"
            ),
        }
    }

    fn contract_mismatch(&mut self) {}

    fn stream_lost(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boser::tests::{Server, TOKEN, WAIT, serve_at, sse, state_then_silence, timing};
    use crate::manager::replace_if_changed;
    use futures::{StreamExt, stream};
    use tokio::sync::{Notify, watch};
    use tokio_util::task::AbortOnDropHandle;

    fn sink(
        initial: Timezone,
    ) -> (
        TimezoneSink<impl Fn(Timezone) -> bool + Send + 'static>,
        watch::Receiver<Timezone>,
    ) {
        let (sender, receiver) = watch::channel(initial);
        (
            TimezoneSink {
                publisher: move |timezone| replace_if_changed(&sender, timezone),
            },
            receiver,
        )
    }

    fn response(id: &str) -> TimezoneResponse {
        TimezoneResponse {
            id: id.to_owned(),
            label: "ignored label".to_owned(),
            offset: "+00:00".to_owned(),
        }
    }

    #[test]
    fn response_decodes_the_boser_wire_shape() {
        let response: TimezoneResponse =
            serde_json::from_str(r#"{"id":"Europe/Prague","label":"Prague","offset":"+02:00"}"#)
                .expect("BUG: the Boser timezone response must decode");

        assert_eq!(response.id, "Europe/Prague");
        assert_eq!(response.label, "Prague");
        assert_eq!(response.offset, "+02:00");
    }

    #[test]
    fn valid_event_publishes_its_iana_id() {
        let (mut sink, state) = sink(Timezone::default());

        sink.observe(&response("Europe/Prague"));

        assert_eq!(state.borrow().iana(), "Europe/Prague");
    }

    #[test]
    fn an_unrepresentable_zone_keeps_the_last_valid_one() {
        let initial = Timezone::from_str("Europe/Prague").expect("BUG: valid test timezone");
        let (mut sink, state) = sink(initial);

        sink.observe(&response("not/a-zone"));

        assert_eq!(state.borrow().iana(), "Europe/Prague");
    }

    #[test]
    fn a_repeated_zone_wakes_no_receiver() {
        let (mut sink, mut state) = sink(Timezone::default());
        sink.observe(&response("Europe/Prague"));
        state.mark_unchanged();

        sink.observe(&response("Europe/Prague"));

        assert!(
            !state
                .has_changed()
                .expect("BUG: the sink keeps the sender alive"),
            "widgets would redraw for a zone they already show"
        );
    }

    fn event(id: &str) -> String {
        format!("data: {{\"id\":\"{id}\",\"label\":\"ignored\",\"offset\":\"+00:00\"}}\n\n")
    }

    async fn wait_for_zone(zone: &mut watch::Receiver<Timezone>, iana: &str) {
        tokio::time::timeout(WAIT, zone.wait_for(|zone| zone.iana() == iana))
            .await
            .unwrap_or_else(|_elapsed| panic!("{iana} was never published"))
            .expect("BUG: the observer task keeps the sender alive");
    }

    #[tokio::test]
    async fn a_dropped_stream_keeps_the_zone_until_boser_reports_a_new_one() {
        let server = Server::default();
        let drop_stream = Arc::new(Notify::new());
        let reconnected = Arc::new(Notify::new());
        let address = serve_at(&server, EVENTS_PATH, {
            let drop_stream = drop_stream.clone();
            let reconnected = reconnected.clone();
            move |attempt| {
                let drop_stream = drop_stream.clone();
                let reconnected = reconnected.clone();
                async move {
                    if attempt == 1 {
                        let until_dropped =
                            stream::once(async move { drop_stream.notified().await })
                                .filter_map(|()| std::future::ready(None));
                        sse(stream::iter([event("Europe/Prague")]).chain(until_dropped))
                    } else {
                        reconnected.notified().await;
                        state_then_silence(event("Asia/Tokyo"))
                    }
                }
            }
        })
        .await;
        let dir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
        let token_path = dir.path().join("token");
        std::fs::write(&token_path, TOKEN).expect("BUG: the token file writes");
        let (sink, mut zone) = sink(Timezone::default());
        let _observer = AbortOnDropHandle::new(crate::boser::spawn(
            StreamConfig {
                address,
                token_path,
                timing: timing(),
            },
            sink,
        ));

        wait_for_zone(&mut zone, "Europe/Prague").await;
        drop_stream.notify_one();
        server.wait_for_attempts(2).await;
        assert_eq!(
            zone.borrow().iana(),
            "Europe/Prague",
            "a dropped stream must not reset the zone widgets show"
        );
        reconnected.notify_one();
        wait_for_zone(&mut zone, "Asia/Tokyo").await;
    }
}
