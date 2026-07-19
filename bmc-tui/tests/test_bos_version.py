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

"""Unit tests for the BOS version grammar."""

import re

import pytest

from bmc_tui.bos_version import parse_bos_version


def test_canonical_version_roundtrips() -> None:
    version = parse_bos_version("2025-06-15-0-acde0123-25.06-plus")

    assert version.canonical == "2025-06-15-0-acde0123-25.06-plus"
    assert version.release_date == "2025-06-15"


def test_noncanonical_version_renders_canonically() -> None:
    version = parse_bos_version("2025-06-15-007-ACDE0123-25.6.01")

    assert version.canonical == "2025-06-15-7-acde0123-25.06.1"


@pytest.mark.parametrize(
    "text",
    [
        "2025-06-15-0-acde0123-25.06-plus-rc",
        "2025-06-15-0-acde0123-25.06-rc",
    ],
)
def test_build_suffix_roundtrips(text: str) -> None:
    assert parse_bos_version(text).canonical == text


@pytest.mark.parametrize(
    "text",
    [
        "hello",
        "2022-09-27-0--22.08",
        "2022-09-27-99999-06ba61b5-22.08",
        "2022-09-27-0-06ba61b5-22.08.1.1",
        "2022-13-27-0-06ba61b5-22.08",
        "2022-09-27-0-06ba61b5-22.08.0",
        "2022-09-27-0-06ba61b5-22.08-foo1",
        "0000-01-01-0-06ba61b5-22.08",
        "\N{FULLWIDTH DIGIT TWO}022-09-27-0-06ba61b5-22.08",
    ],
)
def test_invalid_versions_are_rejected(text: str) -> None:
    with pytest.raises(ValueError):
        parse_bos_version(text)


@pytest.mark.parametrize(
    ("text", "reason"),
    [
        ("2022-09-27-0-06ba61b5-22.999", "invalid version month: outside u8 range"),
        ("2022-09-27-0-06ba61b5-22", "invalid version name"),
    ],
)
def test_version_name_rejection_keeps_the_specific_reason(text: str, reason: str) -> None:
    with pytest.raises(ValueError, match=re.escape(reason)):
        parse_bos_version(text)


def test_version_name_ordering() -> None:
    versions = [
        parse_bos_version("2025-05-01-0-06ba61b5-25.05").version,
        parse_bos_version("2025-06-01-0-06ba61b5-25.06").version,
        parse_bos_version("2025-06-01-0-06ba61b5-25.06.1").version,
        parse_bos_version("2025-07-01-0-06ba61b5-25.07").version,
    ]

    assert versions[0] < versions[1] < versions[2] < versions[3]


def test_noncanonical_version_name_is_structurally_equal() -> None:
    noncanonical = parse_bos_version("2025-06-01-0-06ba61b5-25.6").version
    canonical = parse_bos_version("2025-06-01-0-06ba61b5-25.06").version

    assert noncanonical == canonical
