// Copyright (C) 2025  Braiins Systems s.r.o.

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

type ServerCodec = JsonLinesCodec<WidgetMessage, AppMessage>;

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

const BMC_IPC_SOCKET_ENV: &str = "BMC_IPC_SOCKET";
const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_millis(5000);
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(5000);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(5000);

#[derive(Debug)]
pub struct UnixSpawner {
    socket_dir: PathBuf,
    connection_timeout: Duration,
    handshake_timeout: Duration,
}

impl UnixSpawner {
    #[must_use]
    pub fn new(socket_dir: PathBuf) -> Self {
        Self {
            socket_dir,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    fn socket_path(&self, instance_id: Uuid) -> PathBuf {
        self.socket_dir.join(format!("{instance_id}.sock"))
    }

    pub async fn spawn(
        &self,
        widget: &WidgetInfo,
        instance_id: Uuid,
        init_msg: AppMessage,
    ) -> Result<UnixConnection, SpawnError> {
        tokio::fs::create_dir_all(&self.socket_dir)
            .await
            .map_err(SpawnError::CreateSocketDir)?;

        let socket_path = self.socket_path(instance_id);

        let _ = tokio::fs::remove_file(&socket_path).await;

        let listener = UnixListener::bind(&socket_path).map_err(|e| SpawnError::BindSocket {
            path: socket_path.clone(),
            source: e,
        })?;

        let child = Command::new(&widget.binary_path)
            .env(BMC_IPC_SOCKET_ENV, &socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(SpawnError::SpawnProcess)?;

        let stream = timeout(self.connection_timeout, listener.accept())
            .await
            .map_err(|_| SpawnError::ConnectionTimeout(self.connection_timeout))?
            .map_err(|e| SpawnError::BindSocket {
                path: socket_path.clone(),
                source: e,
            })?
            .0;

        let mut connection = UnixConnection::new(stream, child, socket_path);

        connection.send(init_msg).await?;

        let response = timeout(self.handshake_timeout, connection.recv())
            .await
            .map_err(|_| SpawnError::HandshakeTimeout(self.handshake_timeout))?;

        match response? {
            WidgetMessage::Ready => Ok(connection),
            WidgetMessage::Error { message, .. } => Err(SpawnError::WidgetInitError(message)),
            other @ WidgetMessage::Action(_) => Err(SpawnError::UnexpectedHandshakeMessage(other)),
        }
    }

    pub async fn shutdown(&self, connection: &mut UnixConnection) -> Result<(), SpawnError> {
        connection.send(AppMessage::Shutdown).await?;

        if timeout(DEFAULT_SHUTDOWN_TIMEOUT, connection.child.wait())
            .await
            .is_err()
        {
            warn!(
                "widget did not exit within {:?}, killing",
                DEFAULT_SHUTDOWN_TIMEOUT
            );
            connection.child.kill().await.ok();
        }

        // Socket cleanup happens in UnixConnection::drop
        Ok(())
    }
}

pub struct UnixConnection {
    framed: Framed<UnixStream, ServerCodec>,
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

    pub async fn send(&mut self, msg: AppMessage) -> Result<(), SpawnError> {
        self.framed.send(msg).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<WidgetMessage, SpawnError> {
        self.framed
            .next()
            .await
            .ok_or(SpawnError::ConnectionClosed)?
            .map_err(SpawnError::Codec)
    }
}

impl Drop for UnixConnection {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!("failed to remove socket file: {}", e);
            }
        }
    }
}
