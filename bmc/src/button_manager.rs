// Copyright (C) 2025  Braiins Systems s.r.o.
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

// Button manager taken from BOS

use crate::compositor::Compositor;
use crate::manager::BmcManager;
use crate::system_manager::ScreenRequest;
use bmc_button::{ButtonEvent, ButtonId, Buttons};
use bmc_platform::HardwareCapabilities;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_stream::StreamExt;
use tracing::info;
use tracing::log::warn;

/// Maximum hold duration to trigger a reboot (0-2 seconds)
const REBOOT_MAX_HOLD_DURATION: Duration = Duration::from_secs(2);
/// Minimum hold duration to trigger a factory reset (5+ seconds)
const FACTORY_RESET_MIN_HOLD_DURATION: Duration = Duration::from_secs(5);
/// Mirrors boser's `LOCATE_AND_SWAP_SCREEN_MAX_HOLD_DURATION`.
/// A release under it sends the IP-report packet; the same release shows the address here.
const BOSER_REPORT_IP_MAX_HOLD_DURATION: Duration = Duration::from_secs(1);
/// Hold at which the display turns off, while the button is still down.
/// Boser does nothing past its own bound, so this one is the BMC application's alone.
const DISPLAY_OFF_MIN_HOLD_DURATION: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
pub enum ButtonState {
    Up,
    Down {
        pressed: Instant,
    },
    /// The hold already blanked the display: the deadline must not fire again
    /// and the release has nothing left to do.
    DisplayOffRequested,
}

pub struct ButtonManager<T>
where
    T: BmcManager,
{
    pub buttons: Arc<Box<dyn Buttons + Send + Sync>>,
    pub state: HashMap<ButtonId, ButtonState>,
    pub bmc_manager: Arc<T>,
    /// Every handled press writes `Wake` here;
    /// the IP-report hold overwrites it with `Blank` the moment it reaches the display-off bound.
    pub screen_request: watch::Sender<ScreenRequest>,
    pub compositor: Arc<dyn Compositor>,
    pub capabilities: HardwareCapabilities,
}

impl<T> std::fmt::Debug for ButtonManager<T>
where
    T: BmcManager + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ButtonManager")
            .field("buttons", &self.buttons)
            .field("state", &self.state)
            .field("bmc_manager", &self.bmc_manager)
            .finish_non_exhaustive()
    }
}

impl<T> ButtonManager<T>
where
    T: BmcManager,
{
    /// Creates a new `ButtonManager` with the given buttons trait
    pub fn new(
        buttons: Arc<Box<dyn Buttons + Send + Sync>>,
        bmc_manager: Arc<T>,
        screen_request: watch::Sender<ScreenRequest>,
        compositor: Arc<dyn Compositor>,
        capabilities: HardwareCapabilities,
    ) -> Self {
        Self {
            buttons,
            state: HashMap::new(),
            bmc_manager,
            screen_request,
            compositor,
            capabilities,
        }
    }

    fn handles(&self, button: &ButtonId) -> bool {
        match button {
            // Acting on it here too would race boser's own reboot or factory reset.
            ButtonId::Reset => !self.capabilities.boser_managed,
            ButtonId::IpReport => true,
        }
    }

    pub async fn run(mut self) {
        self.manage_buttons().await;
    }

    /// Main function to poll the button events and make actions
    pub async fn manage_buttons(&mut self) {
        let mut stream = self
            .buttons
            .to_stream()
            .expect("BUG: Can't create button stream");

        loop {
            let display_off_at = self.display_off_deadline();
            let display_off_hold = async move {
                match display_off_at {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let event = tokio::select! {
                biased;
                () = display_off_hold => {
                    self.request_display_off();
                    continue;
                }
                event = stream.next() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            info!("New button event: {:?}", event);
            let inner = match event {
                Ok(inner) => inner,
                Err(error) => {
                    warn!("Error while reading button event: {error}");
                    continue;
                }
            };
            let button = match &inner {
                ButtonEvent::Pressed(button) | ButtonEvent::Released(button) => button,
            };
            if !self.handles(button) {
                info!(
                    "Ignoring {inner:?}: not handled on {}",
                    self.capabilities.product_name
                );
                continue;
            }
            match inner {
                ButtonEvent::Pressed(button) => {
                    self.screen_request.send_replace(ScreenRequest::Wake);
                    if let Some(ButtonState::Down { .. } | ButtonState::DisplayOffRequested) =
                        self.state.get(&button)
                    {
                        warn!("Button pressed without being released: {button:?}");
                    }
                    self.state.insert(
                        button,
                        ButtonState::Down {
                            pressed: Instant::now(),
                        },
                    );
                }
                ButtonEvent::Released(button) => {
                    match self.state.insert(button.clone(), ButtonState::Up) {
                        Some(ButtonState::Down { pressed }) => {
                            let held = pressed.elapsed();
                            match &button {
                                ButtonId::Reset => {
                                    self.handle_reset_button(held).await;
                                }
                                ButtonId::IpReport => {
                                    handle_report_ip_button(self.compositor.as_ref(), held);
                                }
                            }
                        }
                        Some(ButtonState::DisplayOffRequested) => {}
                        Some(ButtonState::Up) | None => {
                            warn!("Button released without being pressed: {button:?}");
                        }
                    }
                }
            }
        }
    }

    /// When the IP-report hold reaches the display-off bound,
    /// or `None` while nothing is counting toward it.
    fn display_off_deadline(&self) -> Option<Instant> {
        match self.state.get(&ButtonId::IpReport)? {
            ButtonState::Down { pressed } => Some(*pressed + DISPLAY_OFF_MIN_HOLD_DURATION),
            ButtonState::Up | ButtonState::DisplayOffRequested => None,
        }
    }

    fn request_display_off(&mut self) {
        info!(
            "Requesting the display off (IP-report button held for {} s)",
            DISPLAY_OFF_MIN_HOLD_DURATION.as_secs()
        );
        self.screen_request.send_replace(ScreenRequest::Blank);
        self.state
            .insert(ButtonId::IpReport, ButtonState::DisplayOffRequested);
    }

    /// Reset button has 2 roles: this function handles reboot and factory reset
    /// based on how long the button is held down.
    async fn handle_reset_button(&self, held: Duration) {
        if held <= REBOOT_MAX_HOLD_DURATION {
            info!("Rebooting the system");
            if let Err(e) = self.bmc_manager.reboot().await {
                warn!("Error while rebooting: {e}");
            }
        } else if held >= FACTORY_RESET_MIN_HOLD_DURATION {
            info!("Performing factory reset");
            if let Err(e) = self.bmc_manager.factory_reset(false).await {
                warn!("Error while performing factory reset: {e}");
            }
        } else {
            info!(
                "Reset button pressed for {} seconds (between {}-{}s), ignoring",
                held.as_secs(),
                REBOOT_MAX_HOLD_DURATION.as_secs(),
                FACTORY_RESET_MIN_HOLD_DURATION.as_secs()
            );
        }
    }
}

/// Ask the device-info overlay to show the device address on a short press;
/// what the overlay does with it is in `docs/devel/system-overlays/overlays.md`.
/// The display-off hold acts before its release gets here,
/// so any release past the address bound is ignored.
fn handle_report_ip_button(compositor: &dyn Compositor, held: Duration) {
    if held < BOSER_REPORT_IP_MAX_HOLD_DURATION {
        info!("Reporting the device address on screen");
        if let Err(e) = compositor.broadcast_report_ip() {
            warn!("Error while requesting the address screen: {e}");
        }
    } else {
        info!(
            "IP-report button held for {} ms, past the {} s address bound; ignoring",
            held.as_millis(),
            BOSER_REPORT_IP_MAX_HOLD_DURATION.as_secs()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootloader_config::BootloaderConfig;
    use crate::compositor::testing::RecordingCompositor;
    use crate::manager::{UpgradeError, UpgradeMarker};
    use crate::session;
    use axum_extra::extract::cookie::Cookie;
    use bmc_button::ButtonEventStream;
    use bmc_platform::{BosPlatform, BosVersion, HardwareProfile, Product};
    use bmc_shared_time::time::Timezone;
    use futures::StreamExt as _;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::watch;

    const UNREACHABLE: &str = "BUG: button handling must not reach the manager's other stubs";

    /// A button event and how long the stream waits before yielding it.
    /// Under `start_paused` that wait is the hold duration the manager sees.
    type DelayedEvent = (Duration, ButtonEvent);

    struct StubButtons {
        events: Vec<DelayedEvent>,
        pulled: Arc<AtomicUsize>,
    }

    impl Buttons for StubButtons {
        fn to_stream(&self) -> anyhow::Result<ButtonEventStream> {
            let pulled = self.pulled.clone();
            let events = self.events.clone().into_iter();
            Ok(Box::pin(
                futures::stream::unfold(events, |mut events| async move {
                    let (delay, event) = events.next()?;
                    tokio::time::sleep(delay).await;
                    Some((Ok(event), events))
                })
                .inspect(move |_| {
                    pulled.fetch_add(1, Ordering::SeqCst);
                }),
            ))
        }
    }

    #[derive(Debug, Clone)]
    struct StubSession;

    impl session::Handle for StubSession {
        fn is_valid(&self) -> bool {
            unimplemented!("{UNREACHABLE}")
        }
        fn id(&self) -> String {
            unimplemented!("{UNREACHABLE}")
        }
    }

    #[derive(Debug, Default)]
    struct StubSessionManager;

    #[async_trait::async_trait]
    impl session::Manager for StubSessionManager {
        type Error = std::io::Error;
        type Session = StubSession;
        const SESSION_TIMEOUT: u32 = 0;

        async fn login(&self, _password: &str) -> Result<Cookie<'static>, Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn logout(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn logout_all_related(&self, _session: Self::Session) -> Result<(), Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn extend(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn find(&self, _cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Reboot,
        FactoryReset { hard: bool },
    }

    #[derive(Debug, Default)]
    struct StubManager {
        calls: Mutex<Vec<Call>>,
    }

    impl StubManager {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("BUG: call log poisoned").clone()
        }
    }

    #[async_trait::async_trait]
    impl BmcManager for StubManager {
        type SessionManager = StubSessionManager;
        type Error = std::io::Error;

        async fn version(&self) -> Option<BosVersion> {
            unimplemented!("{UNREACHABLE}")
        }
        fn platform(&self) -> BosPlatform {
            unimplemented!("{UNREACHABLE}")
        }
        fn network_manager(&self) -> &dyn bmc_net::NetworkManager {
            unimplemented!("{UNREACHABLE}")
        }
        async fn upgrade(
            &self,
            _keep_settings: bool,
            _upgrade_image_path: &Path,
            _progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
        ) -> Result<(), UpgradeError> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn consume_upgrade_marker(&self) -> UpgradeMarker {
            unimplemented!("{UNREACHABLE}")
        }
        async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
            unimplemented!("{UNREACHABLE}")
        }
        fn session_manager(&self) -> Self::SessionManager {
            unimplemented!("{UNREACHABLE}")
        }
        async fn check_password(&self, _password: Option<&str>) -> Result<bool, Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn set_password(&self, _password: Option<String>) -> Result<(), Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
        fn timezone(&self) -> Timezone {
            unimplemented!("{UNREACHABLE}")
        }
        async fn set_timezone(&self, _timezone: Timezone) -> anyhow::Result<()> {
            unimplemented!("{UNREACHABLE}")
        }
        fn watch_timezone_updates(&self) -> watch::Receiver<Timezone> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
            self.calls
                .lock()
                .expect("BUG: call log poisoned")
                .push(Call::FactoryReset { hard });
            Ok(())
        }
        async fn reboot(&self) -> anyhow::Result<()> {
            self.calls
                .lock()
                .expect("BUG: call log poisoned")
                .push(Call::Reboot);
            Ok(())
        }
        async fn control_service(&self, _service: &str, _actions: &[&str]) -> anyhow::Result<()> {
            unimplemented!("{UNREACHABLE}")
        }
        async fn handle_graceful_shutdown(&self) {
            unimplemented!("{UNREACHABLE}")
        }
        fn support_archive(&self) -> impl tokio::io::AsyncRead + Send + Unpin + 'static {
            unimplemented!("{UNREACHABLE}");
            #[expect(
                unreachable_code,
                reason = "stub panics on use; the value only pins the RPIT type"
            )]
            return tokio::io::empty();
        }
        async fn sync_boot_environment(
            &self,
            _config: &BootloaderConfig,
        ) -> Result<(), Self::Error> {
            unimplemented!("{UNREACHABLE}")
        }
    }

    struct Harness {
        button_manager: ButtonManager<StubManager>,
        manager: Arc<StubManager>,
        screen_request: watch::Sender<ScreenRequest>,
        pulled: Arc<AtomicUsize>,
        compositor: Arc<RecordingCompositor>,
    }

    const BOSER_OWNS_THE_BUTTON: bool = true;
    const BMC_OWNS_THE_BUTTON: bool = false;

    /// `boser_managed` is the only field the button manager reads; the rest is filler
    /// borrowed from a real profile so no assertion depends on it.
    fn capabilities(boser_managed: bool) -> HardwareCapabilities {
        HardwareCapabilities {
            boser_managed,
            ..HardwareProfile::for_product(Product::Bmc100).capabilities()
        }
    }

    fn harness(boser_managed: bool, events: Vec<DelayedEvent>) -> Harness {
        let manager = Arc::new(StubManager::default());
        let (screen_request, _) = watch::channel(ScreenRequest::Wake);
        let pulled = Arc::new(AtomicUsize::new(0));
        let compositor = Arc::new(RecordingCompositor::default());
        let button_manager = ButtonManager::new(
            Arc::new(Box::new(StubButtons {
                events,
                pulled: pulled.clone(),
            })),
            manager.clone(),
            screen_request.clone(),
            compositor.clone(),
            capabilities(boser_managed),
        );
        Harness {
            button_manager,
            manager,
            screen_request,
            pulled,
            compositor,
        }
    }

    struct Outcome {
        calls: Vec<Call>,
        /// What the buttons last asked of the screen, or `None` when they asked
        /// nothing at all.
        screen_request: Option<ScreenRequest>,
        events_pulled: usize,
        reset_state: Option<ButtonState>,
        report_ip_broadcasts: usize,
    }

    async fn drive(boser_managed: bool, events: Vec<DelayedEvent>) -> Outcome {
        let mut harness = harness(boser_managed, events);
        // Subscribed before the run, so an unwritten channel stays distinguishable
        // from one written back to the value it started on.
        let mut requests = harness.screen_request.subscribe();
        harness.button_manager.manage_buttons().await;
        Outcome {
            calls: harness.manager.calls(),
            screen_request: requests
                .has_changed()
                .expect("BUG: the harness outlives the run and holds the sender")
                .then(|| *requests.borrow_and_update()),
            events_pulled: harness.pulled.load(Ordering::SeqCst),
            reset_state: harness.button_manager.state.get(&ButtonId::Reset).copied(),
            report_ip_broadcasts: harness.compositor.report_ip_broadcast_count(),
        }
    }

    fn press_and_release() -> Vec<DelayedEvent> {
        press_and_hold(ButtonId::Reset, Duration::ZERO)
    }

    fn press_and_hold(button: ButtonId, held: Duration) -> Vec<DelayedEvent> {
        vec![
            (Duration::ZERO, ButtonEvent::Pressed(button.clone())),
            (held, ButtonEvent::Released(button)),
        ]
    }

    fn recorded_a_press(state: Option<&ButtonState>) -> bool {
        match state {
            Some(ButtonState::Down { .. } | ButtonState::DisplayOffRequested) => true,
            Some(ButtonState::Up) | None => false,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_short_press_reboots_when_bmc_owns_the_button() {
        let outcome = drive(BMC_OWNS_THE_BUTTON, press_and_release()).await;
        assert_eq!(
            outcome.calls,
            [Call::Reboot],
            "a release inside the reboot bound has to reboot"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_wakes_the_screen_when_bmc_owns_the_button() {
        let outcome = drive(BMC_OWNS_THE_BUTTON, press_and_release()).await;
        assert_eq!(
            outcome.screen_request,
            Some(ScreenRequest::Wake),
            "a reset press BMC acts on has to wake the screen"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_hold_factory_resets_when_bmc_owns_the_button() {
        let outcome = drive(
            BMC_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::Reset, FACTORY_RESET_MIN_HOLD_DURATION),
        )
        .await;
        assert_eq!(
            outcome.calls,
            [Call::FactoryReset { hard: false }],
            "a hold past the factory-reset bound has to soft-reset the device"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_intermediate_hold_does_nothing() {
        let outcome = drive(
            BMC_OWNS_THE_BUTTON,
            press_and_hold(
                ButtonId::Reset,
                REBOOT_MAX_HOLD_DURATION + Duration::from_secs(1),
            ),
        )
        .await;
        assert_eq!(
            outcome.calls,
            [],
            "a hold between the two bounds has to leave the device alone"
        );
        assert_eq!(
            outcome.screen_request,
            Some(ScreenRequest::Wake),
            "the hold never reached the handler, so the do-nothing window went untested"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_boser_managed_board_leaves_the_reset_button_alone() {
        let outcome = drive(BOSER_OWNS_THE_BUTTON, press_and_release()).await;
        assert_eq!(outcome.calls, [], "BMC acted on the reset button");
        assert_eq!(
            outcome.screen_request, None,
            "BMC woke the screen for the reset button"
        );
        assert!(
            !recorded_a_press(outcome.reset_state.as_ref()),
            "BMC recorded the press, so the filter sits below the state write"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_release_leaves_the_countdown_where_the_press_restarted_it() {
        // A hold inside the do-nothing window: the release has no effect of
        // its own, so the only thing it could do is write to the screen.
        let held = FACTORY_RESET_MIN_HOLD_DURATION.saturating_sub(Duration::from_secs(1));
        let mut harness = harness(BMC_OWNS_THE_BUTTON, press_and_hold(ButtonId::Reset, held));
        let mut requests = harness.screen_request.subscribe();

        tokio::join!(harness.button_manager.manage_buttons(), async {
            requests
                .changed()
                .await
                .expect("BUG: the harness outlives the run and holds the sender");
            assert_eq!(
                *requests.borrow_and_update(),
                ScreenRequest::Wake,
                "the press did not wake the screen"
            );
        });

        assert!(
            !requests
                .has_changed()
                .expect("BUG: the harness outlives the run and holds the sender"),
            "the release wrote to the screen request; a hold counts toward the \
             countdown the press restarted, and only a press restarts it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_boser_managed_board_ignores_a_factory_reset_length_hold() {
        let outcome = drive(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::Reset, FACTORY_RESET_MIN_HOLD_DURATION),
        )
        .await;
        assert_eq!(
            outcome.calls,
            [],
            "BMC wiped the device on a hold boser is also acting on"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_reset_gate_leaves_the_report_ip_button_alone() {
        let outcome = drive(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::IpReport, Duration::ZERO),
        )
        .await;
        assert_eq!(
            outcome.report_ip_broadcasts, 1,
            "the IP-report press was dropped along with the reset button"
        );
        assert_eq!(
            outcome.screen_request,
            Some(ScreenRequest::Wake),
            "the IP-report press did not wake the screen"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn filtered_events_do_not_stop_the_listener() {
        let events = press_and_release();
        let expected = events.len();
        let outcome = drive(BOSER_OWNS_THE_BUTTON, events).await;
        assert_eq!(
            outcome.events_pulled, expected,
            "the listener stopped before draining the stream"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_ip_report_hold_at_the_display_off_bound_requests_the_display_off() {
        let outcome = drive(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::IpReport, DISPLAY_OFF_MIN_HOLD_DURATION),
        )
        .await;
        assert_eq!(
            outcome.screen_request,
            Some(ScreenRequest::Blank),
            "the story promises the blank at three seconds, not only past them"
        );
        assert_eq!(
            outcome.report_ip_broadcasts, 0,
            "a display-off hold must not also show the address"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_display_goes_off_at_the_bound_while_the_button_is_still_held() {
        const RELEASE_AFTER: Duration = Duration::from_secs(10);
        let mut harness = harness(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::IpReport, RELEASE_AFTER),
        );
        let mut requests = harness.screen_request.subscribe();
        let pressed = Instant::now();

        let ((), blanked_after) = tokio::join!(harness.button_manager.manage_buttons(), async {
            tokio::time::timeout(
                RELEASE_AFTER,
                requests.wait_for(|request| *request == ScreenRequest::Blank),
            )
            .await
            .expect("the hold never asked for the blank")
            .expect("BUG: the harness outlives the run and holds the sender");
            pressed.elapsed()
        });

        assert_eq!(
            blanked_after, DISPLAY_OFF_MIN_HOLD_DURATION,
            "the blank waited for something other than the bound"
        );
        assert!(
            !requests
                .has_changed()
                .expect("BUG: the harness outlives the run and holds the sender"),
            "the release wrote to the channel after the hold had blanked the display"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_touch_during_the_hold_wins_over_the_release() {
        const RELEASE_AFTER: Duration = Duration::from_secs(10);
        const TOUCH_AFTER: Duration = Duration::from_secs(5);
        let mut harness = harness(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(ButtonId::IpReport, RELEASE_AFTER),
        );
        let touch = harness.screen_request.clone();
        let mut requests = harness.screen_request.subscribe();

        tokio::join!(harness.button_manager.manage_buttons(), async {
            tokio::time::sleep(TOUCH_AFTER).await;
            assert_eq!(
                *requests.borrow_and_update(),
                ScreenRequest::Blank,
                "the hold had not blanked the display before the touch"
            );
            touch.send_replace(ScreenRequest::Wake);
        });

        assert_eq!(
            *requests.borrow(),
            ScreenRequest::Wake,
            "the release re-blanked a display the touch had woken"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_ip_report_hold_between_the_bounds_does_nothing() {
        let outcome = drive(
            BOSER_OWNS_THE_BUTTON,
            press_and_hold(
                ButtonId::IpReport,
                DISPLAY_OFF_MIN_HOLD_DURATION.saturating_sub(Duration::from_millis(1)),
            ),
        )
        .await;
        assert_eq!(
            outcome.screen_request,
            Some(ScreenRequest::Wake),
            "a hold under the display-off bound asked for the blank"
        );
        assert_eq!(
            outcome.report_ip_broadcasts, 0,
            "a hold past the address bound showed the address"
        );
    }

    #[test]
    fn a_release_under_the_bound_asks_for_the_address_screen() {
        let compositor = RecordingCompositor::default();

        handle_report_ip_button(
            &compositor,
            BOSER_REPORT_IP_MAX_HOLD_DURATION.saturating_sub(Duration::from_millis(1)),
        );

        assert_eq!(compositor.report_ip_broadcast_count(), 1);
    }

    #[test]
    fn a_release_at_the_bound_is_ignored() {
        let compositor = RecordingCompositor::default();

        handle_report_ip_button(&compositor, BOSER_REPORT_IP_MAX_HOLD_DURATION);

        assert_eq!(compositor.report_ip_broadcast_count(), 0);
    }
}
