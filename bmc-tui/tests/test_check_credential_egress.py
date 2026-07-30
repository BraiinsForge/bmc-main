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


def _request(case: ce.Case, *, value: str = SECRET) -> Request:
    return Request(method="GET", path=case.path, query={"v": [value]}, headers={}, body=b"")


def _refusal(text: str) -> str:
    return f"WARN bmc_wasm_runtime: refusing fetch: {text}"


def _no_secret(slot: str) -> str:
    err = f'no secret available for credential slot "{slot}"'
    return _refusal(f"credential placeholder unresolved err={err}")


def _withheld(slot: str) -> str:
    return f'WARN bmc: {ce._WITHHELD} widget_id=w slot="{slot}" reason=x'


def _delivered() -> ce.Outcome:
    case = _case("permitted")
    return ce.Outcome(case=case, served=_request(case))


def _all_refusals_satisfied() -> list[ce.Outcome]:
    return [
        ce.Outcome(case=_case("pinned"), refusal=ce._PIN_REFUSAL),
        ce.Outcome(case=_case("unbound"), refusal=ce._NO_SECRET_REFUSAL),
        ce.Outcome(case=_case("undeclared"), withheld=_withheld("api")),
    ]


def test_url_embeds_the_placeholder_without_inner_spaces() -> None:
    """The log splits on whitespace, so a spaced placeholder would truncate."""
    url = _case("permitted").url(BASE)
    assert "{{credential.weather.token}}" in url
    assert " " not in url


def test_every_case_owns_a_distinct_slot() -> None:
    """Refusals are attributed by slot, so a shared one would cross-credit cases."""
    slots = [c.slot for c in ce.CASES]
    assert len(slots) == len(set(slots))


def test_only_the_pinned_case_uses_a_pinned_credential_type() -> None:
    """A corpus that drifted to unpinned types everywhere would prove nothing."""
    pinned = [c.name for c in ce.CASES if c.account_type == "braiins-pool"]
    assert pinned == ["pinned"]
    assert _case("unbound").account_type is None


def test_the_undeclared_case_binds_a_slot_params_demo_does_not_declare() -> None:
    """Its whole point: config names a real account on a slot the manifest lacks."""
    case = _case("undeclared")
    assert case.account_type is not None
    assert case.slot not in {"pool", "weather", "media"}


def test_collect_attributes_each_request_to_its_own_case() -> None:
    permitted = _case("permitted")
    outcomes = {o.case.name: o for o in ce._collect("", "", [_request(permitted)])}
    assert outcomes["permitted"].served is not None
    assert outcomes["pinned"].served is None, "a request must not credit another case"


def test_collect_separates_two_no_secret_refusals_by_slot() -> None:
    """`unbound` and `undeclared` both surface as a secretless slot."""
    host = "\n".join([_no_secret("media"), _no_secret("api")])
    outcomes = {o.case.name: o for o in ce._collect(host, "", [])}
    assert '"media"' in outcomes["unbound"].refusal
    assert '"api"' in outcomes["undeclared"].refusal


def test_collect_reads_the_withholding_decision_from_the_compositor_log() -> None:
    """The host cannot say why a slot was empty; only the compositor can."""
    outcomes = {o.case.name: o for o in ce._collect("", _withheld("api"), [])}
    assert outcomes["undeclared"].withheld
    assert not outcomes["unbound"].withheld


def test_the_undeclared_case_is_not_satisfied_by_the_host_refusal_alone() -> None:
    """Otherwise it would pass on a never-bound slot, proving nothing about R1."""
    outcomes = {o.case.name: o for o in ce._collect(_no_secret("api"), "", [])}
    assert outcomes["undeclared"].failed


def test_failed_flags_a_secret_that_left_for_a_pinned_host() -> None:
    pinned = _case("pinned")
    assert ce.Outcome(case=pinned, served=_request(pinned)).failed
    assert not ce.Outcome(case=pinned, refusal=ce._PIN_REFUSAL).failed


def test_judge_refuses_to_pass_a_run_where_nothing_fetched() -> None:
    """Every refusal case passes for free when the device never fetched at all."""
    silent = [ce.Outcome(case=c) for c in ce.CASES]
    assert not any(o.served for o in silent)
    with pytest.raises(Abort, match="nothing was proven"):
        ce._judge(silent, "", SECRET)


def test_judge_rejects_a_request_carrying_the_placeholder_instead_of_the_secret() -> None:
    case = _case("permitted")
    unresolved = ce.Outcome(case=case, served=_request(case, value="{{credential.weather.token}}"))
    with pytest.raises(Abort, match="not the resolved secret"):
        ce._judge([unresolved, *_all_refusals_satisfied()], "", SECRET)


def test_judge_rejects_a_secret_that_reached_the_log() -> None:
    """The log must carry the placeholder form; the resolved one is a leak."""
    host = f"fetch completed status=200 body_len=70 url=http://host/x?v={SECRET}"
    with pytest.raises(Abort, match="appeared in the wasm-host log"):
        ce._judge([_delivered(), *_all_refusals_satisfied()], host, SECRET)


def test_judge_passes_a_run_where_only_the_permitted_case_was_sent() -> None:
    ce._judge([_delivered(), *_all_refusals_satisfied()], "", SECRET)
