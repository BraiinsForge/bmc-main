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

from pathlib import Path
from typing import TYPE_CHECKING, cast

import pytest

from bmc_tui.procedures import check_credential_egress as ce
from bmc_tui.server import Request, View
from bmc_tui.stage import Abort

if TYPE_CHECKING:
    from collections.abc import Callable

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
    redirect = _case("redirect")
    pin = _case("account_pin")
    replaces = _case("account_pin_replaces")
    return [
        ce.Outcome(case=_case("pinned"), refusal=ce._PIN_REFUSAL),
        ce.Outcome(case=_case("unbound"), refusal=ce._NO_SECRET_REFUSAL),
        ce.Outcome(case=_case("undeclared"), withheld=_withheld("api")),
        ce.Outcome(case=redirect, served=_request(redirect)),
        ce.Outcome(case=_case("reshaped"), refusal=ce._PIN_REFUSAL),
        ce.Outcome(case=pin, served=_request(pin)),
        ce.Outcome(case=_case("account_pin_denied"), refusal=ce._PIN_REFUSAL),
        ce.Outcome(case=replaces, served=_request(replaces)),
    ]


def test_url_embeds_the_placeholder_without_inner_spaces() -> None:
    """The log splits on whitespace, so a spaced placeholder would truncate."""
    url = _case("permitted").url(BASE)
    assert "{{credential.weather.token}}" in url
    assert " " not in url


def test_every_judged_message_still_exists_in_the_rust_sources() -> None:
    """Each verdict substring-matches a log line the firmware writes.
    A reworded message therefore blinds the harness silently, since these tests
    hold the constants symbolically and stay green either way.
    `_PIN_REFUSAL` drifted exactly so when the pin stopped being the type's alone."""
    root = Path(__file__).resolve().parents[2]
    skip = {"target", ".git", "node_modules", "result"}
    sources = [
        path.read_text(encoding="utf-8", errors="ignore")
        for path in root.rglob("*.rs")
        if not skip & set(path.parts)
    ]
    assert sources, f"no Rust sources under {root}; this guard would pass vacuously"

    for name in ("_PIN_REFUSAL", "_NO_SECRET_REFUSAL", "_WITHHELD"):
        message = cast("str", getattr(ce, name))
        assert any(message in source for source in sources), (
            f"{name} matches no Rust source: {message!r} was reworded or removed"
        )


def test_an_account_pin_reaches_the_stored_account_resolved() -> None:
    """`{authority}` has to resolve to our real host:port, or the pin would name
    a server the request never reaches and the case would pass for the wrong reason."""
    case = _case("account_pin")
    account = ce._account("acc-1", case, SECRET, "host:8000")

    assert account["allow_hosts"] == ["host:8000"]


def test_an_account_without_a_pin_stores_no_allow_hosts_key() -> None:
    """The firmware omits an empty list, so writing one would exercise a shape
    the device never produces."""
    assert "allow_hosts" not in ce._account("acc-1", _case("permitted"), SECRET, "host:8000")


def test_the_replacing_pin_case_targets_a_type_that_pins_elsewhere() -> None:
    """Its whole point is that the account overrides a type with a pin of its own;
    on an unpinned type it would prove only what `account_pin` already does."""
    case = _case("account_pin_replaces")

    assert case.account_type == "braiins-pool"
    assert case.expect == "deliver"
    assert case.allow_hosts("host:8000") == ["host:8000"]


def test_every_refusal_judged_case_owns_a_distinct_slot() -> None:
    """A refusal names a slot, not a URL, so two of them sharing one would
    cross-credit. Cases judged on what arrived are attributed by path instead,
    and may share — `redirect` shares `media` with `unbound`."""
    slots = [c.slot for c in ce.CASES if c.expect not in ce._ARRIVAL_JUDGED]
    assert len(slots) == len(set(slots))


def test_every_case_gets_one_runtime_and_the_carrier_fetches_once() -> None:
    defaults = ce._default_params()
    scenes = [ce._scene(case, BASE, defaults, f"account-{case.name}") for case in ce.CASES]

    assert len(scenes) == len(ce.CASES)
    assert len({scene["id"] for scene in scenes}) == len(ce.CASES)
    scene_widgets = [cast("list[dict[str, object]]", scene["widgets"]) for scene in scenes]
    widgets = [items[0] for items in scene_widgets]
    assert all(len(items) == 1 for items in scene_widgets)
    assert len({widget["id"] for widget in widgets}) == len(ce.CASES)
    params = [cast("dict[str, object]", widget["params"]) for widget in widgets]
    assert {values["string_uri"] for values in params} == {case.url(BASE) for case in ce.CASES}

    carrier = (ce.MANIFEST.parent / "src/lib.rs").read_text(encoding="utf-8")
    assert carrier.count("net::fetch(") == 1, "each runtime must issue one request during init"


def test_only_the_unbound_case_leaves_its_shared_slot_secretless() -> None:
    """`unbound`'s refusal names `media`, so nothing else on `media` may lack a
    binding, or the two would be indistinguishable in the log."""
    secretless = [c.name for c in ce.CASES if c.slot == "media" and c.account_type is None]
    assert secretless == ["unbound"]


def test_the_pinned_type_is_exercised_by_exactly_these_cases() -> None:
    """A corpus that drifted to unpinned types everywhere would prove nothing.
    Spelled out rather than counted, so gaining a pinned case is a decision."""
    pinned = [c.name for c in ce.CASES if c.account_type == "braiins-pool"]
    assert pinned == ["pinned", "reshaped", "account_pin_replaces"]
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


def test_collect_accepts_a_refusal_from_the_delivery_tracing_target() -> None:
    host = _no_secret("media").replace("bmc_wasm_runtime:", "bmc_wasm_runtime::runtime::delivery:")

    outcomes = {o.case.name: o for o in ce._collect(host, "", [])}

    assert '"media"' in outcomes["unbound"].refusal


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
    host = f"fetch succeeded request_id=1 method=GET url=http://host/x?v={SECRET} status=200"
    with pytest.raises(Abort, match="appeared in the wasm-host log"):
        ce._judge([_delivered(), *_all_refusals_satisfied()], host, SECRET)


def test_judge_passes_a_run_where_only_the_permitted_case_was_sent() -> None:
    ce._judge([_delivered(), *_all_refusals_satisfied()], "", SECRET)


def test_a_followed_redirect_fails_the_case() -> None:
    """Reaching the redirect target means the secret went where we never sent it."""
    case = _case("redirect")
    followed = ce.Outcome(
        case=case,
        served=_request(case),
        followed=Request(
            method="GET", path=ce.LANDED_PATH, query={"v": [SECRET]}, headers={}, body=b""
        ),
    )
    assert followed.failed


def test_an_unfollowed_redirect_still_has_to_have_been_fetched() -> None:
    """Otherwise the case passes on a device that never made the request."""
    case = _case("redirect")
    assert ce.Outcome(case=case).failed
    assert not ce.Outcome(case=case, served=_request(case)).failed


def test_the_redirect_view_points_at_the_landing_path() -> None:
    """A 302 the client must not follow, carrying the secret it was sent."""
    seen: list[Request] = []
    case = _case("redirect")
    view = cast("Callable[[Request], View]", ce._views(seen)[case.path])

    bounced = view(_request(case))

    assert bounced.status == 302
    assert bounced.headers["Location"] == f"{ce.LANDED_PATH}?v={SECRET}"
    assert [r.path for r in seen] == [case.path], "the first hop must be recorded"


def test_the_reshaped_case_aims_the_pinned_template_at_the_pinned_host() -> None:
    """A pin reading the template would approve, which is the premise."""
    url = _case("reshaped").url(BASE)
    assert url.startswith("http://{{credential.pool_backup.token}}@api.braiins.com")


def test_the_reshaped_secret_lands_the_request_on_a_recorded_view() -> None:
    """Non-vacuity: a leak has to be a request we can actually see, so the token
    ends the authority early and points the rest at this case's own path."""
    case = _case("reshaped")
    resolved = case.url(BASE).replace(
        f"{{{{credential.{case.slot}.{case.field}}}}}",
        case.secret(SECRET, "host:8000"),
    )

    assert resolved == "http://host:8000/reshaped.png?v=@api.braiins.com/x"
    assert case.path in ce._views([]), "the path a leak would arrive on must record"


def test_the_reshaped_case_does_not_spend_the_canary() -> None:
    """Its secret is an authority, so the run's leak check still means something."""
    assert _case("reshaped").secret(SECRET, "host:8000") != SECRET
    assert _case("pinned").secret(SECRET, "host:8000") == SECRET


def test_collect_reads_a_landing_request_as_a_follow() -> None:
    landed = Request(method="GET", path=ce.LANDED_PATH, query={"v": [SECRET]}, headers={}, body=b"")
    outcomes = {o.case.name: o for o in ce._collect("", "", [landed])}
    assert outcomes["redirect"].followed is landed
    assert outcomes["unbound"].followed is None, "only the redirect case tracks a follow"
