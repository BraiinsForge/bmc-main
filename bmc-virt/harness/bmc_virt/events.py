# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

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
