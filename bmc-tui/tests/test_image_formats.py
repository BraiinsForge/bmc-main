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

"""Unit tests for the image-format harness log parsing and verdicts."""

from pathlib import Path
from typing import TYPE_CHECKING, cast

import pytest

from bmc_tui.nix import StorePath
from bmc_tui.procedures import image_formats as fmt
from bmc_tui.server import Request, ResponseValue
from bmc_tui.stage import Abort

if TYPE_CHECKING:
    from collections.abc import Callable

BASE = "http://host:8000"


def _case(
    name: str,
    *,
    expect: fmt.Expect = "decode",
    body: bytes = b"fixture",
) -> fmt.Case:
    return fmt.Case(
        name=name,
        fmt="PNG",
        file=f"{name}.png",
        magic="89504e47",
        expect=expect,
        make=lambda: body,
    )


def _request(case: fmt.Case, *, range_header: bool = False) -> Request:
    headers = {"Range": "bytes=0-15"} if range_header else {}
    return Request(method="GET", path=f"/{case.file}", query={}, headers=headers, body=b"")


def _failure(case: fmt.Case, *, status: int, body_len: int) -> str:
    return (
        f"fetch failed request_id=1 method=GET url={case.url(BASE)} "
        f"status={status} outcome=Some(Network) body_len={body_len} admission=first"
    )


def _probe(w: int, h: int, data_len: int) -> str:
    return f"host_image_probe {w}x{h} px={w * h} data_len={data_len}"


def _decode(w: int, h: int, data_len: int) -> str:
    return f"host_decode_image {w}x{h} data_len={data_len} decode_us=1500 vmrss_delta_kb=+64"


def test_collect_pairs_probe_and_decode_with_the_recorded_request() -> None:
    case = _case("png", body=b"x" * 42)
    window = "\n".join([_probe(8, 4, 42), _decode(8, 4, 42)])
    (out,) = fmt._collect(window, [case], BASE, [_request(case)])
    assert (out.status, out.fetched) == (200, 42)
    assert (out.probed, out.decoded) == ("8x4", "8x4")
    assert out.decode_us == 1500


def test_collect_does_not_attribute_a_decode_to_the_wrong_case() -> None:
    """Body length is not an identity: two sources can share one."""
    first = _case("first", body=b"x" * 99)
    second = _case("second", body=b"y" * 99)
    outcomes = {
        o.case.name: o
        for o in fmt._collect(
            _decode(1000, 1000, 99),
            [first, second],
            BASE,
            [_request(first), _request(second)],
        )
    }
    decoded = [name for name, o in outcomes.items() if o.decoded]
    assert decoded != ["second"], "a shared length must not silently credit the last fetch"


def test_collect_does_not_treat_the_preflight_range_as_a_device_fetch() -> None:
    case = _case("png", body=b"x" * 42)

    (out,) = fmt._collect("", [case], BASE, [_request(case, range_header=True)])

    assert (out.status, out.fetched) == (None, None)


def test_collect_overrides_a_recorded_request_with_an_admitted_failure() -> None:
    case = _case("large", expect="reject-body", body=b"x" * 42)

    (out,) = fmt._collect(
        _failure(case, status=fmt.FETCH_BODY_TOO_LARGE, body_len=17),
        [case],
        BASE,
        [_request(case)],
    )

    assert (out.status, out.fetched) == (fmt.FETCH_BODY_TOO_LARGE, 17)


def test_collect_attributes_an_error_by_length_without_a_completion_log() -> None:
    case = _case("broken", body=b"x" * 42)
    error = "host_decode_image: decoder rejected input  data_len=42"

    (out,) = fmt._collect(error, [case], BASE, [_request(case)])

    assert out.error == "host_decode_image: decoder rejected input"


def test_collect_does_not_attribute_an_error_with_an_ambiguous_length() -> None:
    first = _case("first", body=b"x" * 42)
    second = _case("second", body=b"y" * 42)
    error = "host_decode_image probe: unknown format  data_len=42"

    outcomes = fmt._collect(error, [first, second], BASE, [_request(first), _request(second)])

    assert not any(out.error for out in outcomes)


def test_views_record_every_selected_case_and_preserve_file_streaming() -> None:
    generated = _case("generated", body=b"generated")
    shipped = next(case for case in fmt.CASES if case.make is None)
    requests: list[Request] = []
    views = fmt._views([generated, shipped], requests)

    generated_view = cast("Callable[[Request], ResponseValue]", views[f"/{generated.file}"])
    shipped_view = cast("Callable[[Request], ResponseValue]", views[f"/{shipped.file}"])
    generated_response = generated_view(_request(generated))
    shipped_response = shipped_view(_request(shipped))

    assert generated_response == b"generated"
    assert shipped_response == Path(fmt.FIXTURES / shipped.file)
    assert [request.path for request in requests] == [f"/{generated.file}", f"/{shipped.file}"]


def test_verdict_separates_a_refused_body_from_a_dead_network() -> None:
    """The host sends a FetchOutcome wire value, not only HTTP codes."""
    network = fmt.Outcome(case=_case("net"), status=0)
    too_large = fmt.Outcome(case=_case("big", expect="reject-body"), status=1000)
    assert _verdict_of(network) != _verdict_of(too_large)
    assert "too large" in _verdict_of(too_large).lower()


def _verdict_of(out: fmt.Outcome) -> str:
    return fmt._verdict(out)


def test_verdict_reports_a_decode_and_a_size_rejection() -> None:
    decoded = fmt.Outcome(case=_case("ok"), status=200, decoded="8x4")
    assert "decoded 8x4" in _verdict_of(decoded)
    rejected = fmt.Outcome(case=_case("big", expect="reject-size"), status=200, probed="3000x3000")
    assert "rejected at 3000x3000" in _verdict_of(rejected)


def test_failed_flags_a_decode_that_never_ran_and_a_rejection_that_decoded() -> None:
    assert fmt.Outcome(case=_case("ok"), status=200).failed
    assert not fmt.Outcome(case=_case("ok"), status=200, decoded="8x4").failed
    reject = _case("big", expect="reject-size")
    assert fmt.Outcome(case=reject, status=200, decoded="8x4", probed="8x4").failed
    assert not fmt.Outcome(case=reject, status=200, probed="8x4").failed


def test_select_runs_every_case_including_the_adversarial_ones() -> None:
    assert fmt._select([]) == list(fmt.CASES)
    assert {c.expect for c in fmt._select([])} >= {"decode", "reject-size", "reject-body"}
    picked = fmt._select([fmt.CASES[0].name])
    assert [c.name for c in picked] == [fmt.CASES[0].name]
    with pytest.raises(Abort):
        fmt._select(["no-such-case"])


def test_preflight_refuses_a_corpus_that_cannot_be_attributed() -> None:
    """Collection keys on body length, so duplicate sizes make results ambiguous."""
    with pytest.raises(Abort, match=r"(?i)ambiguous"):
        fmt._require_distinct_sizes([("a", 7), ("b", 7)])


def test_every_case_has_a_fixture_that_matches_its_magic() -> None:
    """Guards the packaging trap: the fixtures ship with the repo, not the CLI."""
    assert fmt.FIXTURES.is_dir(), fmt.FIXTURES
    for case in fmt.CASES:
        assert case.body()[:8].hex().startswith(case.magic), case.name


def test_fixture_sizes_are_distinct_so_results_can_be_attributed() -> None:
    sizes = [(c.name, len(c.body())) for c in fmt.CASES]
    fmt._require_distinct_sizes(sizes)


def test_synthesised_fixtures_land_on_the_sides_of_the_caps_they_probe() -> None:
    """Each fails one limit and clears the other; a body over the fetch cap
    never reaches a decoder, so one file cannot test both."""
    fetch_cap = 10 * 1024 * 1024
    over_pixels = fmt.flat_png(3000, 3000)
    assert 3000 * 3000 > 4_194_304, "must exceed the decode budget"
    assert len(over_pixels) < fetch_cap, "must clear the fetch cap to reach the decoder"

    assert len(fmt.flat_bmp(2000, 2000)) > fetch_cap, "must be refused before any decoder"


def test_host_build_pin_accepts_the_expected_package(monkeypatch: pytest.MonkeyPatch) -> None:
    reports: list[str] = []
    monkeypatch.setattr(fmt.console, "kv", lambda *_args: None)
    monkeypatch.setattr(fmt.console, "ok", reports.append)
    monkeypatch.setattr(fmt.console, "warn", reports.append)
    expected = StorePath("/nix/store/aaa-bmc-wasm-host")

    fmt._check_host_build(expected, StorePath(f"{expected}/bin/bmc-wasm-host"))

    assert reports == [f"{fmt.PROCESS} is the build this tree expects"]


def test_host_build_pin_warns_about_another_package(monkeypatch: pytest.MonkeyPatch) -> None:
    reports: list[str] = []
    monkeypatch.setattr(fmt.console, "kv", lambda *_args: None)
    monkeypatch.setattr(fmt.console, "ok", reports.append)
    monkeypatch.setattr(fmt.console, "warn", reports.append)

    fmt._check_host_build(
        StorePath("/nix/store/aaa-bmc-wasm-host"),
        StorePath("/nix/store/bbb-bmc-wasm-host/bin/bmc-wasm-host"),
    )

    assert len(reports) == 1
    assert "measurements will not describe this tree" in reports[0]


@pytest.mark.parametrize("case", fmt.CASES, ids=lambda c: c.name)
def test_every_case_ships_the_fixture_it_declares(case: fmt.Case) -> None:
    """The corpus is data this repo owns, so its integrity is checkable here.

    Decoding it is not: that needs the device's decoders, and the harness run
    does it per format. This guards the step before — that the bytes we serve
    are present, real, and the format each case claims.
    """
    if case.make is None:
        path = fmt.FIXTURES / case.file
        assert path.is_file(), f"{path} is missing"
        assert not path.read_bytes()[:16].startswith(b"version https://git-lfs"), (
            f"{case.file} is an LFS pointer — run `git lfs pull`"
        )

    head = case.body()[:16]
    assert head.hex().startswith(case.magic), (
        f"{case.file} starts with {head.hex()[: len(case.magic)]}, not {case.magic}"
    )
