// Copyright (C) 2025  Braiins Systems s.r.o.

use std::future::Future;
use std::path::PathBuf;

use bmc_ipc::{AppMessage, WidgetMessage};
use uuid::Uuid;

use crate::WidgetInfo;

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

    #[error("connection timeout: widget did not connect within {0}ms")]
    ConnectionTimeout(u64),

    #[error("handshake timeout: widget did not send ready within {0}ms")]
    HandshakeTimeout(u64),

    #[error("unexpected message during handshake: expected ready, got {0:?}")]
    UnexpectedHandshakeMessage(WidgetMessage),

    #[error("widget reported error during init: {0}")]
    WidgetInitError(String),

    #[error("failed to send message: {0}")]
    SendError(std::io::Error),

    #[error("failed to receive message: {0}")]
    RecvError(std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(#[from] bmc_ipc::ProtocolError),

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
