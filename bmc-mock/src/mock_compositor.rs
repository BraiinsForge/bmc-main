// Copyright (C) 2025  Braiins Systems s.r.o.

//! Mock compositor for x86 development without Wayland.
//!
//! Logs all compositor operations instead of rendering to a display.

use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, SettingUpdate,
    Size, WidgetAction,
};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct MockCompositor {
    action_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    event_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<CompositorEvent>>>,
    display_name: std::sync::Mutex<Option<String>>,
}

impl MockCompositor {
    #[must_use]
    pub fn new() -> Self {
        let (_action_tx, action_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            action_rx: std::sync::Mutex::new(Some(action_rx)),
            event_tx,
            event_rx: std::sync::Mutex::new(Some(event_rx)),
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
        pid: Option<u32>,
    ) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: register widget '{}' at ({},{}) size {}x{} pid={:?}",
            instance_id,
            position.x,
            position.y,
            size.width,
            size.height,
            pid,
        );
        // Immediately signal that the widget is ready
        let _ = self
            .event_tx
            .send(CompositorEvent::WidgetReady { instance_id });
        Ok(())
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: unregister widget '{}'", instance_id);
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

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: broadcast setting {:?}", setting);
        Ok(())
    }

    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction> {
        self.action_rx
            .lock()
            .expect("BUG: action_rx lock poisoned")
            .take()
            .expect("BUG: action_receiver already taken")
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
