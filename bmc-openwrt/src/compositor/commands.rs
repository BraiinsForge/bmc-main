// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::compositor::{CompositorEvent, InstanceId, Position, SceneLayout, Size, WidgetAction};
use bmc_widget_protocol::{SettingUpdate, WidgetInitialConfig};

#[derive(Debug)]
pub enum CompositorCommand {
    RegisterWidget {
        instance_id: InstanceId,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
        /// Signalled once the command has been fully applied. The coordinator
        /// waits on this before spawning so that the widget's first Wayland
        /// request reliably resolves to the registered instance.
        ack: flume::Sender<()>,
    },
    SetWidgetPid {
        instance_id: InstanceId,
        pid: u32,
        ack: flume::Sender<()>,
    },
    UnregisterWidget {
        instance_id: InstanceId,
    },
    ClearPid {
        pid: u32,
    },
    SetActiveScene {
        layout: SceneLayout,
    },
    SetSceneCycling {
        scenes: Vec<SceneLayout>,
    },
    SetActiveSceneIndex {
        index: usize,
    },
    BroadcastSetting {
        setting: SettingUpdate,
    },
    /// Push fresh params to a running widget without respawning it.
    /// Only valid when geometry (size) is unchanged; the widget keeps
    /// its EGL surface and Slint scene and re-reads its manifest
    /// options from the new `params` event.
    UpdateWidgetParams {
        instance_id: InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    },
    Shutdown,
}

#[derive(Debug)]
pub enum CompositorResponse {
    Started { wayland_display: String },
    Event(CompositorEvent),
    Action(WidgetAction),
    ShutdownComplete,
    Error { message: String },
}
