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

"""Unit tests for the credential-egress verdicts and their anti-vacuity guard."""

import pytest

from bmc_tui.procedures import check_credential_egress as ce
from bmc_tui.server import Request
from bmc_tui.stage import Abort

BASE = "http://host:8000"
SECRET = "canary-deadbeef"


def _case(name: str) -> ce.Case:
    return next(c for c in ce.CASES if c.name == name)


def _request(case: ce.Case, *, token: str = SECRET) -> Request:
    return Request(
        method="GET",
        path=case.path,
        query={ce.FIELD: [token]},
        headers={},
        body=b"",
    )


def _refusal(text: str) -> str:
    return f"WARN bmc_wasm_runtime: refusing fetch: {text}"


def _delivered() -> ce.Outcome:
    case = _case("permitted")
    return ce.Outcome(case=case, served=_request(case))


def test_url_embeds_the_placeholder_without_inner_spaces() -> None:
    """The log splits on whitespace, so a spaced placeholder would truncate."""
    url = _case("permitted").url(BASE)
    assert "{{credential.api.token}}" in url
    assert " " not in url


def test_collect_attributes_each_request_to_its_own_case() -> None:
    permitted, pinned = _case("permitted"), _case("pinned")
    outcomes = {o.case.name: o for o in ce._collect("", [_request(permitted)])}
    assert outcomes["permitted"].served is not None
    assert outcomes["pinned"].served is None, "a request must not credit another case"
    assert pinned.path != permitted.path


def test_collect_reads_each_refusal_kind_into_the_case_that_expects_it() -> None:
    window = "\n".join([_refusal(ce._PIN_REFUSAL), _refusal(ce._UNRESOLVED_REFUSAL)])
    outcomes = {o.case.name: o for o in ce._collect(window, [])}
    assert ce._PIN_REFUSAL in outcomes["pinned"].refusal
    assert ce._UNRESOLVED_REFUSAL in outcomes["unbound"].refusal
    assert ce._PIN_REFUSAL not in outcomes["unbound"].refusal
    assert not outcomes["permitted"].refusal, "the delivered case has no refusal to explain"


def test_a_refusal_of_the_wrong_kind_does_not_satisfy_a_case() -> None:
    """Both refusals name the same slot; only the message tells them apart."""
    outcomes = {o.case.name: o for o in ce._collect(_refusal(ce._UNRESOLVED_REFUSAL), [])}
    assert outcomes["pinned"].failed


def test_failed_flags_a_secret_that_left_for_a_pinned_host() -> None:
    pinned = _case("pinned")
    assert ce.Outcome(case=pinned, served=_request(pinned)).failed
    assert not ce.Outcome(case=pinned, refusal=ce._PIN_REFUSAL).failed


def test_judge_refuses_to_pass_a_run_where_nothing_fetched() -> None:
    """Every refusal case passes for free when the device never fetched at all."""
    silent = [ce.Outcome(case=c) for c in ce.CASES]
    refusals = [o for o in silent if o.case.expect != "deliver"]
    assert not any(o.served for o in silent)
    with pytest.raises(Abort, match="nothing was proven"):
        ce._judge(silent, "", SECRET)
    assert refusals, "the guard exists because these would otherwise look green"


def test_judge_rejects_a_request_carrying_the_placeholder_instead_of_the_secret() -> None:
    case = _case("permitted")
    unresolved = ce.Outcome(case=case, served=_request(case, token="{{credential.api.token}}"))
    with pytest.raises(Abort, match="not the resolved secret"):
        ce._judge([unresolved, *_refused_cases()], "", SECRET)


def test_judge_rejects_a_secret_that_reached_the_log() -> None:
    """The log must carry the placeholder form; the resolved one is a leak."""
    window = f"fetch completed status=200 body_len=70 url=http://host/x?token={SECRET}"
    with pytest.raises(Abort, match="appeared in the wasm-host log"):
        ce._judge([_delivered(), *_refused_cases()], window, SECRET)


def test_judge_passes_a_run_where_only_the_permitted_case_was_sent() -> None:
    window = "\n".join([_refusal(ce._PIN_REFUSAL), _refusal(ce._UNRESOLVED_REFUSAL)])
    ce._judge([_delivered(), *_refused_cases()], window, SECRET)


def _refused_cases() -> list[ce.Outcome]:
    return [
        ce.Outcome(case=_case("pinned"), refusal=ce._PIN_REFUSAL),
        ce.Outcome(case=_case("unbound"), refusal=ce._UNRESOLVED_REFUSAL),
    ]


def test_only_the_pinned_case_uses_a_pinned_credential_type() -> None:
    """A run proves nothing if every case happens to use an unpinned type."""
    pinned = [c.name for c in ce.CASES if c.type_id == ce.PINNED_TYPE]
    assert pinned == ["pinned"]
    assert _case("permitted").type_id == ce.UNPINNED_TYPE
    assert _case("unbound").type_id is None
