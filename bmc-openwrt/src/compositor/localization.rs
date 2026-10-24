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

//! `deck_localization_v1` server side.
//! The offered languages, then the device language, are replayed to every overlay that binds
//! and pushed on every change;
//! a language an overlay asks for waits here until the loop forwards it to bmc.

use ::deck_localization_v1::server::deck_localization_v1::{self, DeckLocalizationV1};
use bmc_widget_protocol::Language;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

#[derive(Debug, Default)]
pub struct LocalizationState {
    pub resources: Vec<DeckLocalizationV1>,
    /// Whether bmc has announced the language, so a binder is replayed `language`.
    language_announced: bool,
    /// The language announced last; `None` while the device speaks none it offers.
    language: Option<Language>,
    /// The offered tags as the `languages` event carries them, replayed like the language.
    offered: Option<String>,
    /// The tag an overlay asked for since the loop last took it; the newest request wins.
    pending: Option<String>,
    /// Whether a request awaits its answer: the next announcement goes out
    /// even when the language stays, since that is what settles the asker's choice.
    answer_owed: bool,
}

impl LocalizationState {
    pub fn set_language(&mut self, language: Option<Language>) {
        let answer_owed = std::mem::take(&mut self.answer_owed);
        if self.language_announced && self.language == language && !answer_owed {
            return;
        }
        self.resources.retain(Resource::is_alive);
        for resource in &self.resources {
            resource.language(language.map(Language::tag));
        }
        self.language_announced = true;
        self.language = language;
    }

    pub fn set_languages(&mut self, languages: &[Language]) {
        let tags = languages
            .iter()
            .map(|language| language.tag())
            .collect::<Vec<_>>()
            .join(" ");
        if self.offered.as_ref() == Some(&tags) {
            return;
        }
        self.resources.retain(Resource::is_alive);
        for resource in &self.resources {
            resource.languages(tags.clone());
        }
        self.offered = Some(tags);
    }

    pub fn remove(&mut self, resource: &DeckLocalizationV1) {
        self.resources.retain(|r| r != resource);
    }

    pub fn take_request(&mut self) -> Option<String> {
        self.pending.take()
    }
}

impl GlobalDispatch<DeckLocalizationV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckLocalizationV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        if let Some(tags) = &state.localization.offered {
            resource.languages(tags.clone());
        }
        if state.localization.language_announced {
            resource.language(state.localization.language.map(Language::tag));
        }
        state.localization.resources.push(resource);
    }
}

impl Dispatch<DeckLocalizationV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckLocalizationV1,
        request: deck_localization_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_localization_v1::Request::SetLanguage { value } => {
                tracing::info!(tag = %value, "overlay asked for a language");
                state.localization.pending = Some(value);
                state.localization.answer_owed = true;
            }
            deck_localization_v1::Request::Destroy => state.localization.remove(resource),
            other => tracing::warn!("Unknown deck_localization_v1 request: {other:?}"),
        }
    }
}

pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckLocalizationV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use ::deck_localization_v1::client::deck_localization_v1 as client_api;
    use smithay::reexports::wayland_server::Display;
    use wayland_client::protocol::{wl_callback, wl_registry};
    use wayland_client::{Connection, EventQueue, QueueHandle};

    use super::{Language, create_global};
    use crate::compositor::state::{ClientState, CompositorState};

    #[derive(Default)]
    struct TestClient {
        localization: Option<client_api::DeckLocalizationV1>,
        languages: Vec<Option<String>>,
        offered: Vec<String>,
        /// Every event's name, in the order it arrived.
        heard: Vec<&'static str>,
    }

    impl wayland_client::Dispatch<wl_registry::WlRegistry, ()> for TestClient {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            (): &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
                && interface == "deck_localization_v1"
            {
                state.localization = Some(registry.bind::<client_api::DeckLocalizationV1, _, _>(
                    name,
                    version.min(1),
                    qh,
                    (),
                ));
            }
        }
    }

    impl wayland_client::Dispatch<wl_callback::WlCallback, ()> for TestClient {
        fn event(
            _: &mut Self,
            _: &wl_callback::WlCallback,
            _: wl_callback::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl wayland_client::Dispatch<client_api::DeckLocalizationV1, ()> for TestClient {
        fn event(
            state: &mut Self,
            _: &client_api::DeckLocalizationV1,
            event: client_api::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                client_api::Event::Language { value } => {
                    state.heard.push("language");
                    state.languages.push(value);
                }
                client_api::Event::Languages { tags } => {
                    state.heard.push("languages");
                    state.offered.push(tags);
                }
                other => panic!("BUG: unexpected deck_localization_v1 event {other:?}"),
            }
        }
    }

    /// A compositor of a device that offers its languages, no client attached yet.
    fn compositor() -> (Display<CompositorState>, CompositorState) {
        let display: Display<CompositorState> =
            Display::new().expect("BUG: test Wayland display should initialize");
        let compositor = CompositorState::new(
            &display,
            480,
            1280,
            480,
            1280,
            60_000,
            "test-seat",
            &bmc_platform::HardwareProfile::for_product(bmc_platform::Product::Bmc100),
        );
        create_global(&display.handle());
        (display, compositor)
    }

    struct Bound {
        conn: Connection,
        queue: EventQueue<TestClient>,
        client: TestClient,
    }

    impl Bound {
        /// Dispatch one round and return the languages announced in it alone.
        fn collect_next(
            &mut self,
            display: &mut Display<CompositorState>,
            compositor: &mut CompositorState,
        ) -> Vec<Option<String>> {
            self.client.languages.clear();
            sync_round(
                display,
                compositor,
                &self.conn,
                &mut self.queue,
                &mut self.client,
            );
            std::mem::take(&mut self.client.languages)
        }
    }

    /// Bind from a fresh client, leaving what the bind replayed in `client.languages`.
    fn bind(display: &mut Display<CompositorState>, compositor: &mut CompositorState) -> Bound {
        let (server_stream, client_stream) =
            UnixStream::pair().expect("BUG: unix socket pair should be creatable");
        display
            .handle()
            .insert_client(server_stream, Arc::new(ClientState::default()))
            .expect("BUG: test client stream should be insertable into a fresh display");
        let conn = Connection::from_socket(client_stream)
            .expect("BUG: test client socket should form a valid connection");
        let mut queue: EventQueue<TestClient> = conn.new_event_queue();
        let mut client = TestClient::default();

        conn.display().get_registry(&queue.handle(), ());
        sync_round(display, compositor, &conn, &mut queue, &mut client);
        assert!(
            client.localization.is_some(),
            "BUG: deck_localization_v1 global should have been advertised"
        );
        sync_round(display, compositor, &conn, &mut queue, &mut client);

        Bound {
            conn,
            queue,
            client,
        }
    }

    /// One request/response round, closed by a sync's `done`,
    /// so a round the server has nothing to say in still returns.
    fn sync_round(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        conn.display().sync(&queue.handle(), ());
        conn.flush()
            .expect("BUG: test client flush should succeed on a live socket pair");
        display
            .dispatch_clients(compositor)
            .expect("BUG: test server dispatch should succeed on a live socket pair");
        display
            .flush_clients()
            .expect("BUG: test server flush should succeed on a live socket pair");
        queue
            .blocking_dispatch(client)
            .expect("BUG: test client dispatch should succeed once the server has replied");
    }

    /// `language` event values as the client records them, null ones as `None`.
    fn tags(values: &[Option<&str>]) -> Vec<Option<String>> {
        values.iter().map(|tag| tag.map(str::to_owned)).collect()
    }

    #[test]
    fn a_late_binder_is_replayed_the_language_announced_last() {
        let (mut display, mut compositor) = compositor();
        compositor.localization.set_language(Some(Language::ZH));
        compositor.localization.set_language(Some(Language::EN_US));

        let bound = bind(&mut display, &mut compositor);

        assert_eq!(bound.client.languages, tags(&[Some("en-US")]));
    }

    #[test]
    fn a_language_no_longer_offered_is_announced_and_replayed_as_null() {
        let (mut display, mut compositor) = compositor();
        compositor.localization.set_language(Some(Language::ZH));
        let mut bound = bind(&mut display, &mut compositor);

        compositor.localization.set_language(None);

        assert_eq!(
            bound.collect_next(&mut display, &mut compositor),
            tags(&[None])
        );
        let late = bind(&mut display, &mut compositor);
        assert_eq!(late.client.languages, tags(&[None]));
    }

    #[test]
    fn a_late_binder_hears_the_offered_languages_before_the_language() {
        let (mut display, mut compositor) = compositor();
        compositor
            .localization
            .set_languages(&[Language::EN_GB, Language::EN_US, Language::ZH]);
        compositor.localization.set_language(Some(Language::ZH));

        let bound = bind(&mut display, &mut compositor);

        assert_eq!(bound.client.offered, ["en-GB en-US zh"]);
        assert_eq!(bound.client.heard, ["languages", "language"]);
    }

    #[test]
    fn an_unchanged_list_of_languages_is_not_announced_again() {
        let (mut display, mut compositor) = compositor();
        let mut bound = bind(&mut display, &mut compositor);

        compositor
            .localization
            .set_languages(&[Language::EN_GB, Language::ZH]);
        compositor
            .localization
            .set_languages(&[Language::EN_GB, Language::ZH]);
        let _ = bound.collect_next(&mut display, &mut compositor);

        assert_eq!(bound.client.offered, ["en-GB zh"]);
    }

    #[test]
    fn a_binder_before_any_announcement_hears_the_first_one() {
        let (mut display, mut compositor) = compositor();
        let mut bound = bind(&mut display, &mut compositor);
        assert_eq!(bound.client.languages, tags(&[]));

        compositor.localization.set_language(Some(Language::ZH));

        assert_eq!(
            bound.collect_next(&mut display, &mut compositor),
            tags(&[Some("zh")])
        );
    }

    #[test]
    fn an_unchanged_language_is_not_announced_again() {
        let (mut display, mut compositor) = compositor();
        compositor.localization.set_language(Some(Language::ZH));
        let mut bound = bind(&mut display, &mut compositor);

        compositor.localization.set_language(Some(Language::ZH));

        assert_eq!(bound.collect_next(&mut display, &mut compositor), tags(&[]));
    }

    #[test]
    fn a_request_that_changes_nothing_is_answered_once() {
        let (mut display, mut compositor) = compositor();
        compositor.localization.set_language(Some(Language::ZH));
        let mut bound = bind(&mut display, &mut compositor);
        bound
            .client
            .localization
            .as_ref()
            .expect("BUG: bound above")
            .set_language("xx".to_owned());
        bound.collect_next(&mut display, &mut compositor);

        compositor.localization.set_language(Some(Language::ZH));
        assert_eq!(
            bound.collect_next(&mut display, &mut compositor),
            tags(&[Some("zh")]),
            "bmc refused it and announced what it speaks"
        );
        compositor.localization.set_language(Some(Language::ZH));
        assert_eq!(
            bound.collect_next(&mut display, &mut compositor),
            tags(&[]),
            "the answer is spent"
        );
    }

    #[test]
    fn a_language_an_overlay_asks_for_waits_until_the_loop_takes_it_once() {
        let (mut display, mut compositor) = compositor();
        let mut bound = bind(&mut display, &mut compositor);

        bound
            .client
            .localization
            .as_ref()
            .expect("BUG: bound above")
            .set_language("zh".to_owned());
        bound.collect_next(&mut display, &mut compositor);

        assert_eq!(
            compositor.localization.take_request().as_deref(),
            Some("zh")
        );
        assert_eq!(compositor.localization.take_request(), None);
    }
}
