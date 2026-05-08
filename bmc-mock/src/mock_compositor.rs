// Copyright (C) 2025  Braiins Systems s.r.o.

//! Mock compositor for x86 development without Wayland.
//!
//! Logs all compositor operations instead of rendering to a display.

use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, SettingUpdate,
    Size, WidgetAction, WidgetInitialConfig, WidgetRequestStatus,
};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct MockCompositor {
    action_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    event_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<CompositorEvent>>>,
    status_tx: mpsc::UnboundedSender<WidgetRequestStatus>,
    display_name: std::sync::Mutex<Option<String>>,
}

impl MockCompositor {
    #[must_use]
    pub fn new() -> Self {
        let (_action_tx, action_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (status_tx, mut status_rx) = mpsc::unbounded_channel::<WidgetRequestStatus>();
        tokio::spawn(async move {
            while let Some(status) = status_rx.recv().await {
                tracing::info!(
                    "MockCompositor: led_request_status widget={} req={} status={:?}",
                    status.instance_id,
                    status.request_id,
                    status.status
                );
            }
        });
        Self {
            action_rx: std::sync::Mutex::new(Some(action_rx)),
            event_tx,
            event_rx: std::sync::Mutex::new(Some(event_rx)),
            status_tx,
            display_name: std::sync::Mutex::new(None),
        }
    }
}

impl Default for MockCompositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor for MockCompositor {
    fn start(&self) -> Result<String, CompositorError> {
        let name = "wayland-mock-0".to_owned();
        tracing::info!("MockCompositor: started (display={})", name);
        *self
            .display_name
            .lock()
            .expect("BUG: display_name lock poisoned") = Some(name.clone());
        Ok(name)
    }

    fn wayland_display(&self) -> Option<String> {
        self.display_name
            .lock()
            .expect("BUG: display_name lock poisoned")
            .clone()
    }

    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: register widget '{}' at ({},{}) size {}x{} initial={:?}",
            instance_id,
            position.x,
            position.y,
            size.width,
            size.height,
            initial_config,
        );
        // Immediately signal that the widget is ready
        let _ = self
            .event_tx
            .send(CompositorEvent::WidgetReady { instance_id });
        Ok(())
    }

    fn set_widget_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set_widget_pid '{}' pid={}",
            instance_id,
            pid
        );
        Ok(())
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: unregister widget '{}'", instance_id);
        Ok(())
    }

    fn clear_pid(&self, pid: u32) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: clear_pid {pid}");
        Ok(())
    }

    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set active scene with {} widgets",
            layout.widgets.len()
        );
        for w in &layout.widgets {
            tracing::debug!(
                "  widget '{}': ({},{}) {}x{} visible={}",
                w.instance_id,
                w.position.x,
                w.position.y,
                w.size.width,
                w.size.height,
                w.visible,
            );
        }
        Ok(())
    }

    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set scene cycling with {} scenes",
            scenes.len()
        );
        Ok(())
    }

    fn set_active_scene_index(&self, index: usize) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: set active scene index {}", index);
        Ok(())
    }

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: broadcast setting {:?}", setting);
        Ok(())
    }

    fn update_widget_params(
        &self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: update_widget_params {instance_id}: {}",
            serde_json::Value::Object(params)
        );
        Ok(())
    }

    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction> {
        self.action_rx
            .lock()
            .expect("BUG: action_rx lock poisoned")
            .take()
            .expect("BUG: action_receiver already taken")
    }

    fn request_status_sender(&self) -> mpsc::UnboundedSender<WidgetRequestStatus> {
        self.status_tx.clone()
    }

    fn event_receiver(&self) -> mpsc::UnboundedReceiver<CompositorEvent> {
        self.event_rx
            .lock()
            .expect("BUG: event_rx lock poisoned")
            .take()
            .expect("BUG: event_receiver already taken")
    }

    fn shutdown(&self) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: shutdown");
        Ok(())
    }
}
