// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::process::Child;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use super::coordinator::WidgetEnv;
use super::{
    LinkerConfig, PathDiscovery, SpawnError, WaylandSpawner, WidgetDiscovery, WidgetRegistry,
};

const DEFAULT_XDG_RUNTIME_DIR: &str = "/tmp/run";

#[derive(Debug)]
pub struct WidgetManager {
    registry: WidgetRegistry,
    spawner: WaylandSpawner,
    children: Arc<RwLock<HashMap<String, Child>>>,
}

impl WidgetManager {
    pub async fn init(widgets_paths: Vec<PathBuf>, linker: Option<LinkerConfig>) -> Self {
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

        let spawner = match linker {
            Some(config) => {
                info!(
                    linker = %config.linker_path,
                    "using dynamic linker for widget spawning"
                );
                WaylandSpawner::with_linker(config)
            }
            None => WaylandSpawner::new(),
        };

        Self {
            registry,
            spawner,
            children: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn spawn_widget(&self, widget_uid: Uuid, env: WidgetEnv) -> Result<(), SpawnError> {
        let widget = self.registry.get(&widget_uid).ok_or_else(|| {
            SpawnError::SpawnProcess(Error::new(
                ErrorKind::NotFound,
                format!("widget not found: {widget_uid}"),
            ))
        })?;

        info!(
            "spawning widget '{}' instance {}",
            widget.manifest.name, env.instance_id
        );

        let xdg_runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| DEFAULT_XDG_RUNTIME_DIR.to_owned());

        let child = self.spawner.spawn(widget, &env, &xdg_runtime_dir)?;

        self.children
            .write()
            .await
            .insert(env.instance_id.clone(), child);

        info!("widget instance {} spawned", env.instance_id);

        Ok(())
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        let mut children = self.children.write().await;
        if let Some(mut child) = children.remove(instance_id) {
            // Try graceful termination first via SIGTERM, then force kill
            if let Err(e) = child.kill().await {
                warn!("failed to kill widget {}: {}", instance_id, e);
            }
            info!("stopped widget instance {}", instance_id);
        } else {
            warn!("attempted to stop unknown widget instance {}", instance_id);
        }
    }

    pub async fn stop_all(&self) {
        let mut children = self.children.write().await;
        for (instance_id, mut child) in children.drain() {
            if let Err(e) = child.kill().await {
                warn!("failed to kill widget {}: {}", instance_id, e);
            }
        }
        info!("all widgets stopped");
    }
}
