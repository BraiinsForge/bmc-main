// Copyright (C) 2025  Braiins Systems s.r.o.

//! Generic IPC helpers for widgets.

use bmc_ipc::{AppMessage, SettingUpdate, Settings, SizeInfo};
use serde::de::DeserializeOwned;

use crate::client::{ClientError, WidgetClient};

/// Connects to the IPC socket and handles the Init handshake.
/// Returns the client, size info, deserialized params, and settings.
pub async fn connect_widget<P>()
-> Result<(WidgetClient, SizeInfo, P, Settings), Box<dyn std::error::Error + Send + Sync>>
where
    P: DeserializeOwned,
{
    let mut client = WidgetClient::connect().await?;

    let init_msg = client.recv().await?;
    let AppMessage::Init {
        size,
        params,
        settings,
    } = init_msg
    else {
        client
            .send_error("expected Init message".to_owned(), false)
            .await?;
        return Err("expected Init message".into());
    };

    let params: P = serde_json::from_value(params)?;

    Ok((client, size, params, settings))
}

/// Runs the message loop, calling handlers for settings updates and shutdown.
pub async fn run_message_loop<F, G>(
    mut client: WidgetClient,
    mut settings_handler: F,
    mut shutdown_handler: G,
) where
    F: FnMut(SettingUpdate) + Send + 'static,
    G: FnMut() + Send + 'static,
{
    loop {
        match client.recv().await {
            Ok(AppMessage::SettingsUpdate { update }) => {
                settings_handler(update);
            }
            Ok(AppMessage::Shutdown) => {
                shutdown_handler();
                break;
            }
            Ok(AppMessage::Init { .. }) => {
                // Ignore duplicate Init messages
            }
            Err(ClientError::ConnectionClosed) => {
                break;
            }
            Err(e) => {
                tracing::error!("error receiving message: {}", e);
                break;
            }
        }
    }
}
