// Copyright (C) 2025  Braiins Systems s.r.o.

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use bmc_ipc::{AppMessage, CodecError, JsonLinesCodec, WidgetMessage};
use futures::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tokio_util::codec::Framed;
use tracing::warn;
use uuid::Uuid;

use super::WidgetInfo;

/// Codec type for server-side IPC (decodes WidgetMessage, encodes AppMessage).
type ServerCodec = JsonLinesCodec<WidgetMessage, AppMessage>;

/// Error that can occur during widget spawning or communication.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("failed to create socket directory: {0}")]
    CreateSocketDir(std::io::Error),

    #[error("failed to bind socket at '{path}': {source}")]
    BindSocket {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to spawn process: {0}")]
    SpawnProcess(std::io::Error),

    #[error("connection timeout: widget did not connect within {0:?}")]
    ConnectionTimeout(Duration),

    #[error("handshake timeout: widget did not send ready within {0:?}")]
    HandshakeTimeout(Duration),

    #[error("unexpected message during handshake: expected ready, got {0:?}")]
    UnexpectedHandshakeMessage(WidgetMessage),

    #[error("widget reported error during init: {0}")]
    WidgetInitError(String),

    #[error("codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("connection closed")]
    ConnectionClosed,
}

/// Trait for spawning widget processes.
pub trait ProcessSpawner {
    /// The connection type returned after spawning.
    type Connection: WidgetConnection;

    /// Spawn a new widget process and return a connection handle.
    fn spawn(
        &self,
        widget: &WidgetInfo,
        instance_id: Uuid,
        init_msg: AppMessage,
    ) -> impl Future<Output = Result<Self::Connection, SpawnError>> + Send;
}

/// Trait for communicating with a spawned widget.
pub trait WidgetConnection: Send {
    /// Send a message to the widget.
    fn send(&mut self, msg: AppMessage) -> impl Future<Output = Result<(), SpawnError>> + Send;

    /// Receive a message from the widget.
    fn recv(&mut self) -> impl Future<Output = Result<WidgetMessage, SpawnError>> + Send;
}

/// Environment variable name for the IPC socket path.
const BMC_IPC_SOCKET_ENV: &str = "BMC_IPC_SOCKET";

/// Default timeout for widget connection.
const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_millis(5000);

/// Default timeout for widget handshake.
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(5000);

/// Unix socket based widget spawner.
#[derive(Debug)]
pub struct UnixSpawner {
    socket_dir: PathBuf,
    connection_timeout: Duration,
    handshake_timeout: Duration,
}

impl UnixSpawner {
    /// Create a new Unix spawner with the given socket directory.
    #[must_use]
    pub fn new(socket_dir: PathBuf) -> Self {
        Self {
            socket_dir,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// Set the connection timeout.
    #[must_use]
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    /// Set the handshake timeout.
    #[must_use]
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    fn socket_path(&self, instance_id: Uuid) -> PathBuf {
        self.socket_dir.join(format!("{instance_id}.sock"))
    }
}

impl ProcessSpawner for UnixSpawner {
    type Connection = UnixConnection;

    async fn spawn(
        &self,
        widget: &WidgetInfo,
        instance_id: Uuid,
        init_msg: AppMessage,
    ) -> Result<Self::Connection, SpawnError> {
        // Create socket directory if needed
        tokio::fs::create_dir_all(&self.socket_dir)
            .await
            .map_err(SpawnError::CreateSocketDir)?;

        let socket_path = self.socket_path(instance_id);

        // Remove stale socket if exists
        let _ = tokio::fs::remove_file(&socket_path).await;

        // Bind the socket
        let listener = UnixListener::bind(&socket_path).map_err(|e| SpawnError::BindSocket {
            path: socket_path.clone(),
            source: e,
        })?;

        // Spawn the widget process
        let child = Command::new(&widget.binary_path)
            .env(BMC_IPC_SOCKET_ENV, &socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(SpawnError::SpawnProcess)?;

        // Wait for widget to connect
        let stream = timeout(self.connection_timeout, listener.accept())
            .await
            .map_err(|_| SpawnError::ConnectionTimeout(self.connection_timeout))?
            .map_err(|e| SpawnError::BindSocket {
                path: socket_path.clone(),
                source: e,
            })?
            .0;

        let mut connection = UnixConnection::new(stream, child, socket_path);

        // Send init message
        connection.send(init_msg).await?;

        // Wait for ready message
        let response = timeout(self.handshake_timeout, connection.recv())
            .await
            .map_err(|_| SpawnError::HandshakeTimeout(self.handshake_timeout))?;

        match response? {
            WidgetMessage::Ready => Ok(connection),
            WidgetMessage::Error { message, .. } => Err(SpawnError::WidgetInitError(message)),
            other @ WidgetMessage::Action(_) => Err(SpawnError::UnexpectedHandshakeMessage(other)),
        }
    }
}

/// Unix socket connection to a widget process.
pub struct UnixConnection {
    framed: Framed<UnixStream, ServerCodec>,
    #[expect(dead_code)]
    child: Child,
    socket_path: PathBuf,
}

impl std::fmt::Debug for UnixConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnixConnection")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl UnixConnection {
    fn new(stream: UnixStream, child: Child, socket_path: PathBuf) -> Self {
        Self {
            framed: Framed::new(stream, ServerCodec::default()),
            child,
            socket_path,
        }
    }
}

impl WidgetConnection for UnixConnection {
    async fn send(&mut self, msg: AppMessage) -> Result<(), SpawnError> {
        self.framed.send(msg).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<WidgetMessage, SpawnError> {
        self.framed
            .next()
            .await
            .ok_or(SpawnError::ConnectionClosed)?
            .map_err(SpawnError::Codec)
    }
}

impl Drop for UnixConnection {
    fn drop(&mut self) {
        // Clean up socket file
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!("failed to remove socket file: {}", e);
            }
        }
    }
}
