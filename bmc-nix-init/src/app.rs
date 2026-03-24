// Copyright (C) 2026  Braiins Systems s.r.o.

use std::sync::{Arc, Mutex};

use slint::ComponentHandle as _;
use tokio::sync::mpsc::UnboundedSender;

use crate::InitWindow;
use crate::config::InitConfig;
use crate::display::SlintObserver;
use crate::init::{InitPlatform, run_init};
use crate::state::{InitState, InitStateObserver as _};

/// Run the init application with a Slint display.
///
/// This is the shared entry point for both the openwrt and mock binaries.
/// The caller must set up the Slint platform (DRM or virtual display)
/// before calling this function.
pub fn run_app<P: InitPlatform + 'static>(config: InitConfig, platform: Arc<P>) {
    let window = InitWindow::new().expect("BUG: failed to create init window");
    window
        .window()
        .set_size(slint::PhysicalSize::new(1280, 480));
    window.set_status_text("Initializing...".into());
    let observer = SlintObserver::new(&window);
    let weak = window.as_weak();

    // Wrap senders in Arc<Mutex<Option<…>>> so Slint callbacks (registered once)
    // can be reconnected to fresh channels each time the retry loop iterates.
    let retry_tx: Arc<Mutex<Option<UnboundedSender<()>>>> = Arc::new(Mutex::new(None));
    let reconfigure_tx: Arc<Mutex<Option<UnboundedSender<()>>>> = Arc::new(Mutex::new(None));

    let rt = retry_tx.clone();
    window.on_retry_download(move || {
        tracing::info!("retry download requested by user");
        if let Some(tx) = rt.lock().expect("BUG: retry_tx mutex poisoned").as_ref() {
            let _ = tx.send(());
        }
    });

    let rc = reconfigure_tx.clone();
    window.on_reconfigure_wifi(move || {
        tracing::info!("reconfigure WiFi requested by user");
        if let Some(tx) = rc
            .lock()
            .expect("BUG: reconfigure_tx mutex poisoned")
            .as_ref()
        {
            let _ = tx.send(());
        }
    });

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("BUG: failed to create tokio runtime");
        rt.block_on(async {
            loop {
                // Create fresh channels for this iteration
                let (rc_tx, reconfigure_rx) = tokio::sync::mpsc::unbounded_channel();
                let (rt_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();

                // Install fresh senders so Slint callbacks reach this iteration
                *reconfigure_tx
                    .lock()
                    .expect("BUG: reconfigure_tx mutex poisoned") = Some(rc_tx);
                *retry_tx.lock().expect("BUG: retry_tx mutex poisoned") = Some(rt_tx);

                match run_init(
                    &config,
                    platform.clone(),
                    &observer,
                    reconfigure_rx,
                    retry_rx,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("initialization complete");
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = weak.upgrade() {
                                let _ = w.hide();
                            }
                            let _ = slint::quit_event_loop();
                        });
                        break;
                    }
                    Err(e) => {
                        tracing::error!("initialization failed: {e}");

                        // Clear stale senders — button presses between now and
                        // installing fresh senders are silently ignored rather
                        // than sent into a dropped receiver.
                        *reconfigure_tx
                            .lock()
                            .expect("BUG: reconfigure_tx mutex poisoned") = None;
                        *retry_tx.lock().expect("BUG: retry_tx mutex poisoned") = None;

                        observer.on_state_change(&InitState::Error {
                            message: e.to_string(),
                            retryable: true,
                            reconfigurable: e.is_network_related(),
                        });

                        // Create one-shot channels to wait for user action
                        let (wait_rc_tx, mut wait_rc_rx) = tokio::sync::mpsc::unbounded_channel();
                        let (wait_rt_tx, mut wait_rt_rx) = tokio::sync::mpsc::unbounded_channel();
                        *reconfigure_tx
                            .lock()
                            .expect("BUG: reconfigure_tx mutex poisoned") = Some(wait_rc_tx);
                        *retry_tx.lock().expect("BUG: retry_tx mutex poisoned") = Some(wait_rt_tx);

                        tokio::select! {
                            _ = wait_rt_rx.recv() => {
                                tracing::info!("retrying after fatal error");
                            }
                            _ = wait_rc_rx.recv() => {
                                tracing::info!("reconfiguring WiFi after fatal error");
                            }
                        }
                    }
                }
            }
        });
    });

    slint::run_event_loop().expect("BUG: slint event loop failed");
}
