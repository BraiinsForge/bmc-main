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

"""The client must speak Boser's wire contract and turn every refusal into a readable Abort."""

import pytest

from bmc_tui.boser_rest import BoserRest
from bmc_tui.stage import Abort
from tests import boser_stub
from tests.boser_stub import BoserStub, Script


def _client(stub: BoserStub, password: str = "") -> BoserRest:
    return BoserRest(f"http://{stub.address}", password, timeout=5.0)


def test_walkthrough_sends_the_token_and_reads_the_stream_to_completion() -> None:
    script = Script(password="secret")
    with BoserStub(script) as stub:
        client = _client(stub, "secret")
        client.login()
        assert [p["name"] for p in client.installable_packages()] == ["weather"]
        assert client.check(["weather"])["offer"]["id"] == boser_stub.OFFER_ID
        client.start(boser_stub.OFFER_ID)
        events = list(client.events(deadline=float("inf")))
    assert [e["state"] for e in events] == ["RUNNING", "RUNNING", "COMPLETED"]
    assert events[1]["download"]["downloaded_bytes"] == 524_288
    methods = [(method, path) for method, path, _token, _body in script.requests]
    assert methods == [
        ("POST", "/api/v1/auth/login"),
        ("GET", "/api/v1/upgrade/packages/installable"),
        ("POST", "/api/v1/upgrade/check"),
        ("POST", "/api/v1/upgrade/start"),
        ("GET", "/api/v1/upgrade/state/events"),
    ]
    assert script.requests[0][3] == {"username": "root", "password": "secret"}
    assert script.requests[2][3] == {"packages": ["weather"]}
    assert script.requests[3][3] == {"offer_id": boser_stub.OFFER_ID}
    assert all(token == boser_stub.TOKEN for _m, _p, token, _b in script.requests[1:])


def test_wrong_password_is_a_readable_abort() -> None:
    with (
        BoserStub(Script(password="secret")) as stub,
        pytest.raises(Abort, match=r"POST /api/v1/auth/login: HTTP 401 UNAUTHORIZED: wrong"),
    ):
        _client(stub, "nope").login()


def test_unauthenticated_calls_are_refused_before_reaching_the_device_logic() -> None:
    script = Script()
    with BoserStub(script) as stub, pytest.raises(Abort, match="HTTP 401 UNAUTHORIZED"):
        _client(stub).check([])
    assert [path for _m, path, _t, _b in script.requests] == ["/api/v1/upgrade/check"]


def test_busy_start_surfaces_the_error_code() -> None:
    with BoserStub(Script(start_status=409)) as stub:
        client = _client(stub)
        client.login()
        with pytest.raises(Abort, match="HTTP 409 BUSY: an upgrade is running"):
            client.start(boser_stub.OFFER_ID)


def test_stale_offer_is_not_found() -> None:
    with BoserStub(Script()) as stub:
        client = _client(stub)
        client.login()
        with pytest.raises(Abort, match="HTTP 404 EXPIRED"):
            client.start("0000")


def test_stream_ending_without_a_terminal_state_aborts() -> None:
    with BoserStub(Script(events=[boser_stub.RUNNING])) as stub:
        client = _client(stub)
        client.login()
        with pytest.raises(Abort, match="ended before a terminal state"):
            list(client.events(deadline=float("inf")))


def test_stream_stops_at_the_first_terminal_state() -> None:
    events = [boser_stub.RUNNING, boser_stub.FAILED, boser_stub.COMPLETED]
    with BoserStub(Script(events=events)) as stub:
        client = _client(stub)
        client.login()
        seen = list(client.events(deadline=float("inf")))
    assert [e["state"] for e in seen] == ["RUNNING", "FAILED"]


def test_deadline_is_enforced_between_events() -> None:
    ticks = iter([0.0, 10.0, 20.0])
    with BoserStub(Script(events=[boser_stub.RUNNING, boser_stub.RUNNING])) as stub:
        client = _client(stub)
        client.login()
        with pytest.raises(Abort, match="no terminal state within the deadline"):
            list(client.events(deadline=5.0, clock=lambda: next(ticks)))


def test_a_device_that_accepts_the_connection_but_never_answers_is_a_readable_abort() -> None:
    with BoserStub(Script(stall_seconds=1.0)) as stub:
        client = BoserRest(f"http://{stub.address}", "", timeout=0.1)
        with pytest.raises(Abort, match=r"POST /api/v1/auth/login: the device went quiet"):
            client.login()
