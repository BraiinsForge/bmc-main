// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::compositor::{CompositorEvent, InstanceId, Position, SceneLayout, Size, WidgetAction};
use bmc_widget_protocol::SettingUpdate;

#[derive(Debug)]
pub enum CompositorCommand {
    RegisterWidget {
        instance_id: InstanceId,
        position: Position,
        size: Size,
        pid: Option<u32>,
    },
    UnregisterWidget {
        instance_id: InstanceId,
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
