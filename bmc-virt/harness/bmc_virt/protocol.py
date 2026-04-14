"""JSONL wire protocol — message types, encoding, and decoding.

Shared between the guest-side daemon (server.py) and host-side client (client.py).
Every message is a single JSON object terminated by newline.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from bmc_virt.commands import Cmd
    from bmc_virt.events import Event

# ── Message types ──────────────────────────────────────────────────────────────


class MsgType(StrEnum):
    """Top-level message type on the wire."""

    HELLO = "hello"
    SYNCED = "synced"
    EVENT = "event"
    CMD = "cmd"
    ACK = "ack"
    SHUTDOWN = "shutdown"


PROTOCOL_VERSION = 1


# ── Outgoing message constructors ──────────────────────────────────────────────


def _now() -> str:
    return datetime.now(UTC).isoformat()


@dataclass(frozen=True, slots=True)
class Msg:
    """A protocol message ready to be serialized to JSONL."""

    type: MsgType
    ts: str | None = None
    name: str | None = None
    id: str | None = None
    version: int | None = None
    ok: bool | None = None
    error: str | None = None
    reason: str | None = None
    data: dict[str, Any] = field(default_factory=dict)

    def to_line(self) -> bytes:
        """Serialize to a newline-terminated JSON bytes line."""
        d: dict[str, Any] = {"type": self.type}
        if self.ts is not None:
            d["ts"] = self.ts
        if self.name is not None:
            d["name"] = self.name
        if self.id is not None:
            d["id"] = self.id
        if self.version is not None:
            d["version"] = self.version
        if self.ok is not None:
            d["ok"] = self.ok
        if self.error is not None:
            d["error"] = self.error
        if self.reason is not None:
            d["reason"] = self.reason
        if self.data:
            d["data"] = self.data
        return json.dumps(d, separators=(",", ":")).encode() + b"\n"

    @classmethod
    def from_line(cls, line: bytes) -> Msg:
        """Deserialize from a JSON bytes line."""
        d = json.loads(line)
        return cls(
            type=MsgType(d["type"]),
            ts=d.get("ts"),
            name=d.get("name"),
            id=d.get("id"),
            version=d.get("version"),
            ok=d.get("ok"),
            error=d.get("error"),
            reason=d.get("reason"),
            data=d.get("data", {}),
        )


# ── Server-originated message factories ────────────────────────────────────────


def hello() -> Msg:
    return Msg(type=MsgType.HELLO, version=PROTOCOL_VERSION, ts=_now())


def synced() -> Msg:
    return Msg(type=MsgType.SYNCED, ts=_now())


def event(name: Event, data: dict[str, Any] | None = None) -> Msg:
    return Msg(type=MsgType.EVENT, name=name, ts=_now(), data=data or {})


def ack(
    request_id: str,
    *,
    ok: bool = True,
    data: dict[str, Any] | None = None,
    error: str | None = None,
) -> Msg:
    return Msg(type=MsgType.ACK, id=request_id, ok=ok, ts=_now(), data=data or {}, error=error)


def shutdown(reason: str) -> Msg:
    return Msg(type=MsgType.SHUTDOWN, reason=reason, ts=_now())


# ── Client-originated message factories ────────────────────────────────────────


def cmd(request_id: str, name: Cmd, data: dict[str, Any] | None = None) -> Msg:
    return Msg(type=MsgType.CMD, id=request_id, name=name, data=data or {})
