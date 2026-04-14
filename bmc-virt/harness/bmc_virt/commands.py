"""Command types shared between the guest daemon and host client."""

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any


class Cmd(StrEnum):
    """Commands sent from the host to the guest VM."""

    RR_START = "rr.start"
    RR_STOP = "rr.stop"
    SHELL_EXEC = "shell.exec"
    SERVICE_RESTART = "service.restart"


@dataclass(frozen=True, slots=True)
class Ack:
    """Acknowledgement received from the guest after a command."""

    id: str
    ok: bool
    data: dict[str, Any] = field(default_factory=dict)
    error: str | None = None
