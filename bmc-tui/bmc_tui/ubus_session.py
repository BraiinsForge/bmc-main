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

"""Sessions the BMC application and Boser accept, authorized through SSH root access."""

import json
import shlex
from collections.abc import Iterator
from contextlib import contextmanager

from bmc_tui.device import Device
from bmc_tui.stage import Abort, best_effort


@contextmanager
def ssh_session(dev: Device, *, timeout_s: int) -> Iterator[str]:
    """Yield the id of a logged-in session.

    The BMC application and Boser both accept any live ubus session,
    so root over SSH needs no device password to create one.
    The session is destroyed on exit, and expires after `timeout_s` if that fails.
    """
    created = dev.run(f"ubus call session create {_ubus_args({'timeout': timeout_s})}")
    if created is None:
        msg = "BUG: a session cannot be created under --dry-run"
        raise RuntimeError(msg)
    try:
        session_id = json.loads(created)["ubus_rpc_session"]
    except (json.JSONDecodeError, KeyError, TypeError):
        raise Abort(f"ubus did not create a session: {created}") from None
    try:
        yield session_id
    finally:
        best_effort(
            lambda: dev.run(
                f"ubus call session destroy {_ubus_args({'ubus_rpc_session': session_id})}"
            )
        )


def _ubus_args(args: dict[str, object]) -> str:
    return shlex.quote(json.dumps(args))
