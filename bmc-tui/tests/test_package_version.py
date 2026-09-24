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

import pytest

from bmc_tui.package_version import (
    PackageVersion,
    parse_package_version,
    planner_keeps_installed,
)


def _sorted_by_the_planner(versions: list[str]) -> list[str]:
    def key(raw: str) -> PackageVersion:
        parsed = parse_package_version(raw)
        assert parsed is not None, f"{raw} must parse"
        return parsed

    return sorted(versions, key=key)


def test_precedence_follows_semver() -> None:
    # semver.org §11's own example chain.
    chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.1",
        "1.1.0",
        "2.0.0",
    ]
    assert _sorted_by_the_planner(list(reversed(chain))) == chain


def test_build_metadata_orders_like_the_semver_crate() -> None:
    # The planner's `Version` compares build metadata after precedence.
    chain = ["1.0.0", "1.0.0+0", "1.0.0+00", "1.0.0+1", "1.0.0+10", "1.0.0+a", "1.0.0+a.1"]
    assert _sorted_by_the_planner(list(reversed(chain))) == chain


@pytest.mark.parametrize(
    ("short", "full"),
    [("0.8", "0.8.0"), ("1", "1.0.0"), ("0.8-rc1", "0.8.0-rc1"), ("2.1+b7", "2.1.0+b7")],
)
def test_short_cores_are_padded_like_the_planner(short: str, full: str) -> None:
    assert parse_package_version(short) == parse_package_version(full)


@pytest.mark.parametrize(
    "raw",
    ["1.2.3.4", "", "-rc1", "nightly", "01.2.3", "1.2.3-01", "1..2", "18446744073709551616.0.0"],
)
def test_versions_the_planner_rejects_do_not_parse(raw: str) -> None:
    assert parse_package_version(raw) is None


def test_identifiers_too_long_for_int_still_order_by_value() -> None:
    huge = "9" * 5000
    chain = [f"1.0.0-rc.{huge}", f"1.0.0-rc.1{huge}", "1.0.0", f"1.0.0+{huge}"]
    assert _sorted_by_the_planner(list(reversed(chain))) == chain
    assert parse_package_version(f"{huge}.0.0") is None


def test_the_planner_keeps_a_newer_installed_package() -> None:
    assert planner_keeps_installed("0.1.1-bump", "0.1.0")


@pytest.mark.parametrize(
    ("installed", "served"),
    [("0.1.0", "0.1.1"), ("0.1.0", "0.1.0"), ("0.1.1-bump", "0.1.1"), ("nightly", "0.1.0")],
)
def test_the_planner_moves_to_a_served_package_it_does_not_rank_lower(
    installed: str, served: str
) -> None:
    assert not planner_keeps_installed(installed, served)
