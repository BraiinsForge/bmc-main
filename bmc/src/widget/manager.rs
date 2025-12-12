// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::sync::Arc;

use bmc_ipc::AppMessage;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use super::{
    PathDiscovery, SpawnError, UnixConnection, UnixSpawner, WidgetDiscovery, WidgetRegistry,
};

const DEFAULT_SOCKET_DIR: &str = "/tmp/bmc-widgets";

#[derive(Debug)]
pub struct WidgetManager {
    registry: WidgetRegistry,
    spawner: UnixSpawner,
    connections: Arc<RwLock<HashMap<Uuid, UnixConnection>>>,
}

impl WidgetManager {
    pub async fn init(widgets_paths: Vec<PathBuf>) -> Self {
        info!("initializing widget manager");
        for path in &widgets_paths {
            info!(path = %path.display(), "scanning widget directory");
        }

        let discovery = PathDiscovery::new(widgets_paths);
        let widgets = discovery.discover().await;

        let registry = WidgetRegistry::new(widgets);
        info!(count = registry.len(), "widget discovery complete");

        for widget in registry.list() {
            info!(
                name = %widget.manifest.name,
                version = %widget.manifest.version,
                uid = %widget.manifest.uid,
                "registered widget"
            );
        }

        let spawner = UnixSpawner::new(PathBuf::from(DEFAULT_SOCKET_DIR));

        Self {
            registry,
            spawner,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn spawn_widget(
        &self,
        widget_uid: Uuid,
        instance_id: Uuid,
        init_msg: AppMessage,
    ) -> Result<(), SpawnError> {
        let widget = self.registry.get(&widget_uid).ok_or_else(|| {
            SpawnError::SpawnProcess(Error::new(
                ErrorKind::NotFound,
                format!("widget not found: {widget_uid}"),
            ))
        })?;

        info!(
            "spawning widget '{}' instance {}",
            widget.manifest.name, instance_id
        );

        let connection = self.spawner.spawn(widget, instance_id, init_msg).await?;

        self.connections
            .write()
            .await
            .insert(instance_id, connection);

        info!("widget instance {} connected and ready", instance_id);

        Ok(())
    }

    pub async fn stop_widget(&self, instance_id: Uuid) {
        let mut connections = self.connections.write().await;
        if let Some(mut connection) = connections.remove(&instance_id) {
            if let Err(e) = self.spawner.shutdown(&mut connection).await {
                warn!("failed to send shutdown to widget {}: {}", instance_id, e);
            }
            info!("stopped widget instance {}", instance_id);
        } else {
            warn!("attempted to stop unknown widget instance {}", instance_id);
        }
    }

    pub async fn send_message(&self, instance_id: Uuid, msg: AppMessage) -> Result<(), SpawnError> {
        let mut connections = self.connections.write().await;
        let connection = connections.get_mut(&instance_id).ok_or_else(|| {
            SpawnError::SpawnProcess(Error::new(
                ErrorKind::NotFound,
                format!("widget instance not found: {instance_id}"),
            ))
        })?;

        connection.send(msg).await
    }
}
