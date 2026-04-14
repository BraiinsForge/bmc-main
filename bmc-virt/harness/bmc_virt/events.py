"""Event types shared between the guest daemon and host client."""

from dataclasses import dataclass, field
from datetime import datetime
from enum import StrEnum
from typing import Any


class Event(StrEnum):
    """Events emitted by the guest VM."""

    SETUP_DONE = "setup.done"
    APP_STARTED = "app.started"
    APP_READY = "app.ready"
    WIFI_CONFIGURED = "wifi.configured"
    WIFI_ASSOCIATED = "wifi.associated"
    WIFI_GOT_IP = "wifi.got_ip"
    RELAY_LISTENING = "relay.listening"
    RR_RECORDING = "rr.recording"
    RR_STOPPED = "rr.stopped"
    RR_FAILED = "rr.failed"
    SERVICE_STARTED = "service.started"
    SERVICE_STOPPED = "service.stopped"
    SHUTDOWN = "shutdown"


@dataclass(frozen=True, slots=True)
class ReceivedEvent:
    """An event received from the guest VM."""

    name: Event
    ts: datetime
    data: dict[str, Any] = field(default_factory=dict)
