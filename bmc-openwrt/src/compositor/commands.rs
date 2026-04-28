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
    BroadcastSetting {
        setting: SettingUpdate,
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
