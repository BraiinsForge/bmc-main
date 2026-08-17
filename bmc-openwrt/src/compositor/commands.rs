// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use bmc::compositor::{
    CompositorEvent, InstanceId, Position, SceneCycling, SceneLayout, Size, UpgradeDisplaySnapshot,
    WidgetAction,
};
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
    BindRespawnedPid {
        instance_id: InstanceId,
        pid: u32,
        ack: flume::Sender<()>,
    },
    UnregisterWidget {
        instance_id: InstanceId,
    },
    UnregisterAbandoned {
        instance_id: InstanceId,
    },
    ClearPid {
        instance_id: InstanceId,
        expected_pid: u32,
    },
    SetActiveScene {
        layout: SceneLayout,
    },
    SetSceneCycling {
        scenes: Vec<SceneLayout>,
    },
    SetSceneCyclingConfig {
        config: SceneCycling,
    },
    /// Gate automatic cycling independently of the user's configured
    /// `enabled`.
    SetSceneCyclingSuspended {
        suspended: bool,
    },
    ResetToFirstScene,
    BroadcastSetting {
        setting: SettingUpdate,
    },
    SetBrightness {
        value: u8,
    },
    SetWifiAp {
        ssid: Option<String>,
    },
    SetVolume {
        value: u8,
    },
    SetNightMode {
        active: bool,
        until: Option<String>,
    },
    SetUpgradeState {
        state: UpgradeDisplaySnapshot,
    },
    RestartDeclined {
        reason: String,
    },
    /// Push fresh params to a running widget without respawning it.
    /// Only valid when geometry (size) is unchanged; the widget keeps
    /// its EGL surface and Slint scene and re-reads its manifest
    /// options from the new `params` event.
    UpdateWidgetParams {
        instance_id: InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    },
    /// Push a re-resolved credential set to a running widget.
    UpdateWidgetCredentials {
        instance_id: InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: bmc_widget_protocol::CredentialSecrets,
    },
    Shutdown,
    RingAlarm {
        time: String,
        period: String,
        label: String,
        snooze_allowed: bool,
    },
    StopAlarm,
}

#[derive(Debug)]
pub enum CompositorResponse {
    Started { wayland_display: String },
    Event(CompositorEvent),
    Action(WidgetAction),
    ShutdownComplete,
    Error { message: String },
}
