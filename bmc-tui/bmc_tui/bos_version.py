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

"""Parsing and canonical rendering for BOS firmware versions."""

import re
from dataclasses import dataclass
from datetime import date
from functools import total_ordering
from string import ascii_lowercase

_VERSION_RE = re.compile(
    r"^(\d{4})?-(\d{2})?-(\d{2})?-(\d+)?-([0-9A-Fa-f]{8})?-([.0-9]+)?"
    r"(-plus)?(?:-(\w+))?$",
    re.ASCII,
)
_BUILD_SUFFIXES = frozenset({"rc", "nightly", *ascii_lowercase})
_U8_MAX = 255


@total_ordering
@dataclass(frozen=True)
class VersionName:
    year: int
    month: int
    patch: int | None

    def __lt__(self, other: object) -> bool:
        if not isinstance(other, VersionName):
            return NotImplemented
        return self._ordering_key < other._ordering_key

    def __str__(self) -> str:
        rendered = f"{self.year:02}.{self.month:02}"
        if self.patch is not None:
            rendered += f".{self.patch}"
        return rendered

    @property
    def _ordering_key(self) -> tuple[int, int, int]:
        return (self.year, self.month, self.patch or 0)


@dataclass(frozen=True)
class BosVersion:
    date: date
    day_index: int
    commit: int
    version: VersionName
    is_plus: bool
    build: str | None

    @property
    def canonical(self) -> str:
        rendered = (
            f"{self.date.year}-{self.date.month:02}-{self.date.day:02}-{self.day_index}"
            f"-{self.commit:08x}-{self.version}"
        )
        if self.is_plus:
            rendered += "-plus"
        if self.build is not None:
            rendered += f"-{self.build}"
        return rendered

    @property
    def release_date(self) -> str:
        return self.date.isoformat()


def parse_bos_version(text: str) -> BosVersion:
    match = _VERSION_RE.fullmatch(text)
    if match is None:
        raise _invalid(text, "pattern doesn't match")

    year_text, month_text, day_text, day_index_text, commit_text, version_text = (
        match.group(index) for index in range(1, 7)
    )
    required = (
        ("year", year_text),
        ("month", month_text),
        ("day", day_text),
        ("day index", day_index_text),
        ("commit", commit_text),
        ("version", version_text),
    )
    for name, value in required:
        if value is None:
            raise _invalid(text, f"'{name}' is missing")

    try:
        parsed_date = date(int(year_text), int(month_text), int(day_text))
    except ValueError:
        raise _invalid(text, "invalid date") from None

    day_index = _parse_u8(day_index_text, text, "day index")
    commit = int(commit_text, 16)
    version = _parse_version_name(version_text, text)
    build = match.group(8)
    if build is not None and build not in _BUILD_SUFFIXES:
        raise _invalid(text, "invalid build suffix")

    return BosVersion(
        date=parsed_date,
        day_index=day_index,
        commit=commit,
        version=version,
        is_plus=match.group(7) is not None,
        build=build,
    )


def _parse_version_name(version: str, full_version: str) -> VersionName:
    tokens = version.split(".", maxsplit=2)
    try:
        year_token, month_token, *patch_tokens = tokens
    except ValueError:
        raise _invalid(full_version, "invalid version name") from None
    year = _parse_u8(year_token, full_version, "version year")
    month = _parse_u8(month_token, full_version, "version month")
    patch = _parse_u8(patch_tokens[0], full_version, "version patch") if patch_tokens else None

    if patch == 0:
        raise _invalid(full_version, "invalid version name: patch number is zero")
    return VersionName(year=year, month=month, patch=patch)


def _parse_u8(token: str, full_version: str, name: str) -> int:
    try:
        value = int(token)
    except ValueError:
        raise _invalid(full_version, f"invalid {name}") from None
    if not 0 <= value <= _U8_MAX:
        raise _invalid(full_version, f"invalid {name}: outside u8 range")
    return value


def _invalid(version: str, reason: str) -> ValueError:
    return ValueError(f"invalid format: {version} ({reason})")
