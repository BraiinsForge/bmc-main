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

"""BMC virtual machine event daemon, test harness, and CLI."""

import sys
import types
from time import sleep

from bmc_virt import ui
from bmc_virt.client import DaemonConnectionError, DaemonProtocolError
from bmc_virt.commands import Ack
from bmc_virt.events import Event, ReceivedEvent
from bmc_virt.vm import VM, WaitTimeoutError

# Expected errors: print a clean message instead of a stack trace.
_EXPECTED_ERRORS = (
    WaitTimeoutError,
    DaemonConnectionError,
    DaemonProtocolError,
    KeyboardInterrupt,
)

# Install rich traceback handler for unexpected errors only.
# `install()` returns the *previous* excepthook, not rich's — grab rich's from
# `sys.excepthook` after the call so our wrapper can delegate to it.
try:
    from rich.traceback import install as _install_rich_traceback

    _install_rich_traceback(show_locals=True, width=120)
    _rich_hook = sys.excepthook
except ImportError:
    _rich_hook = sys.__excepthook__


def _excepthook(
    exc_type: type[BaseException],
    exc_value: BaseException,
    exc_tb: types.TracebackType | None,
) -> None:
    if isinstance(exc_value, KeyboardInterrupt):
        ui.warn("Interrupted")
        sys.exit(130)
    if isinstance(exc_value, _EXPECTED_ERRORS):
        ui.error(str(exc_value))
        sys.exit(1)
    _rich_hook(exc_type, exc_value, exc_tb)


sys.excepthook = _excepthook

__all__ = [
    "VM",
    "Ack",
    "Event",
    "ReceivedEvent",
    "WaitTimeoutError",
    "sleep",
    "ui",
]
