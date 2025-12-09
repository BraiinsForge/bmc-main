// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget client SDK for communicating with the main application.

use std::env;
use std::path::PathBuf;

use bmc_ipc::{AppMessage, CodecError, JsonLinesCodec, WidgetMessage};
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::Framed;

/// Environment variable name for the IPC socket path.
const BMC_IPC_SOCKET_ENV: &str = "BMC_IPC_SOCKET";

/// Codec type for client-side IPC (decodes AppMessage, encodes WidgetMessage).
type ClientCodec = JsonLinesCodec<AppMessage, WidgetMessage>;

/// Error that can occur during widget client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("BMC_IPC_SOCKET environment variable not set")]
    MissingSocketEnv,

    #[error("failed to connect to socket at '{path}': {source}")]
    ConnectionFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("connection closed")]
    ConnectionClosed,
}

/// Client for widgets to communicate with the main application.
pub struct WidgetClient {
    framed: Framed<UnixStream, ClientCodec>,
}

impl std::fmt::Debug for WidgetClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetClient").finish_non_exhaustive()
    }
}

impl WidgetClient {
    /// Connect to the main application using the socket path from BMC_IPC_SOCKET env var.
    pub async fn connect() -> Result<Self, ClientError> {
        let socket_path =
            env::var(BMC_IPC_SOCKET_ENV).map_err(|_| ClientError::MissingSocketEnv)?;
        let socket_path = PathBuf::from(socket_path);

        let stream =
            UnixStream::connect(&socket_path)
                .await
                .map_err(|e| ClientError::ConnectionFailed {
                    path: socket_path,
                    source: e,
                })?;

        Ok(Self {
            framed: Framed::new(stream, ClientCodec::default()),
        })
    }

    /// Receive a message from the application.
    pub async fn recv(&mut self) -> Result<AppMessage, ClientError> {
        self.framed
            .next()
            .await
            .ok_or(ClientError::ConnectionClosed)?
            .map_err(ClientError::Codec)
    }

    /// Send a message to the application.
    pub async fn send(&mut self, msg: WidgetMessage) -> Result<(), ClientError> {
        self.framed.send(msg).await?;
        Ok(())
    }

    /// Send a Ready message to indicate successful initialization.
    pub async fn send_ready(&mut self) -> Result<(), ClientError> {
        self.send(WidgetMessage::Ready).await
    }

    /// Send an Error message to the application.
    pub async fn send_error(
        &mut self,
        message: String,
        recoverable: bool,
    ) -> Result<(), ClientError> {
        self.send(WidgetMessage::Error {
            message,
            recoverable,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_ipc::{Settings, SizeInfo, SizeType};
    use tempfile::TempDir;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn missing_env_var_returns_error() {
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::remove_var(BMC_IPC_SOCKET_ENV) };
        let result = WidgetClient::connect().await;
        assert!(matches!(result, Err(ClientError::MissingSocketEnv)));
    }

    #[tokio::test]
    async fn connection_failed_returns_error() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let socket_path = temp_dir.path().join("nonexistent.sock");
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::set_var(BMC_IPC_SOCKET_ENV, &socket_path) };

        let result = WidgetClient::connect().await;
        assert!(matches!(result, Err(ClientError::ConnectionFailed { .. })));
    }

    #[tokio::test]
    async fn connect_to_socket() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let socket_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).expect("BUG: failed to bind socket");
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::set_var(BMC_IPC_SOCKET_ENV, &socket_path) };

        let client_handle = tokio::spawn(async move { WidgetClient::connect().await });

        let (_stream, _addr) = listener.accept().await.expect("BUG: failed to accept");
        let client = client_handle.await.expect("BUG: task panicked");
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn send_and_recv_messages() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let socket_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).expect("BUG: failed to bind socket");
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::set_var(BMC_IPC_SOCKET_ENV, &socket_path) };

        let client_handle = tokio::spawn(async move {
            let mut client = WidgetClient::connect()
                .await
                .expect("BUG: failed to connect");

            // Receive Init message
            let msg = client.recv().await.expect("BUG: failed to recv");
            assert!(matches!(msg, AppMessage::Init { .. }));

            // Send Ready message
            client
                .send_ready()
                .await
                .expect("BUG: failed to send ready");

            client
        });

        let (stream, _addr) = listener.accept().await.expect("BUG: failed to accept");
        let codec: JsonLinesCodec<WidgetMessage, AppMessage> = JsonLinesCodec::default();
        let mut framed = Framed::new(stream, codec);

        // Send Init message
        let init_msg = AppMessage::Init {
            size: SizeInfo {
                name: SizeType::Small,
                width: 100,
                height: 100,
            },
            params: serde_json::json!({}),
            settings: Settings::default(),
        };
        framed
            .send(init_msg)
            .await
            .expect("BUG: failed to send init");

        // Receive Ready message
        let response = framed
            .next()
            .await
            .expect("BUG: no response")
            .expect("BUG: codec error");
        assert!(matches!(response, WidgetMessage::Ready));

        client_handle.await.expect("BUG: client task panicked");
    }

    #[tokio::test]
    async fn send_error_message() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let socket_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).expect("BUG: failed to bind socket");
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::set_var(BMC_IPC_SOCKET_ENV, &socket_path) };

        let client_handle = tokio::spawn(async move {
            let mut client = WidgetClient::connect()
                .await
                .expect("BUG: failed to connect");
            client
                .send_error("test error".to_owned(), true)
                .await
                .expect("BUG: failed to send error");
        });

        let (stream, _addr) = listener.accept().await.expect("BUG: failed to accept");
        let codec: JsonLinesCodec<WidgetMessage, AppMessage> = JsonLinesCodec::default();
        let mut framed = Framed::new(stream, codec);

        let response = framed
            .next()
            .await
            .expect("BUG: no response")
            .expect("BUG: codec error");
        match response {
            WidgetMessage::Error {
                message,
                recoverable,
            } => {
                assert_eq!(message, "test error");
                assert!(recoverable);
            }
            _ => panic!("BUG: expected Error message"),
        }

        client_handle.await.expect("BUG: client task panicked");
    }

    #[tokio::test]
    async fn connection_closed_returns_error() {
        let temp_dir = TempDir::new().expect("BUG: failed to create temp dir");
        let socket_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).expect("BUG: failed to bind socket");
        // SAFETY: Test runs in isolation, no concurrent access to env vars
        unsafe { env::set_var(BMC_IPC_SOCKET_ENV, &socket_path) };

        let client_handle = tokio::spawn(async move {
            let mut client = WidgetClient::connect()
                .await
                .expect("BUG: failed to connect");
            // Try to receive after server closes connection
            client.recv().await
        });

        let (stream, _addr) = listener.accept().await.expect("BUG: failed to accept");
        drop(stream); // Close the connection

        let result = client_handle.await.expect("BUG: client task panicked");
        assert!(matches!(result, Err(ClientError::ConnectionClosed)));
    }
}
