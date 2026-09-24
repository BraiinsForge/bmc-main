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

"""Package version ordering as the bmc-nix planner applies it (`bmc-nix/src/index.rs`)."""

import re

# semver.org's recommended pattern, which the Rust `semver` crate enforces too.
_NUMBER = r"0|[1-9][0-9]*"
_PRE_ID = rf"{_NUMBER}|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*"
_SEMVER = re.compile(
    rf"({_NUMBER})\.({_NUMBER})\.({_NUMBER})"
    rf"(?:-((?:{_PRE_ID})(?:\.(?:{_PRE_ID}))*))?"
    r"(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?"
)

_CORE_PARTS = 3
_U64_MAX = 2**64 - 1
_U64_DIGITS = len(str(_U64_MAX))

# One identifier as (kind, digit count, text, spelled length): digit runs
# sort by value without `int()`, which refuses 4300+ digits the crate takes,
# then the shorter spelling first (`0 < 00 < 1`), all before alphanumerics.
_Id = tuple[int, int, str, int]
_Ids = tuple[_Id, ...]
PackageVersion = tuple[int, int, int, _Ids, _Ids]

# A release sorts after every pre-release of its core.
_RELEASE: _Ids = ((2, 0, "", 0),)


def _ids_key(ids: str) -> _Ids:
    def key(i: str) -> _Id:
        if i.isdigit():
            value = i.lstrip("0")
            return (0, len(value), value, len(i))
        return (1, 0, i, 0)

    return tuple(key(i) for i in ids.split("."))


def _parse_semver(raw: str) -> PackageVersion | None:
    match = _SEMVER.fullmatch(raw)
    if match is None:
        return None
    major, minor, patch, pre, build = match.groups()
    core = (major, minor, patch)
    if any(len(n) > _U64_DIGITS or int(n) > _U64_MAX for n in core):
        return None
    return (
        int(major),
        int(minor),
        int(patch),
        _ids_key(pre) if pre is not None else _RELEASE,
        _ids_key(build) if build is not None else (),
    )


def parse_package_version(raw: str) -> PackageVersion | None:
    """Mirror of the planner's parser: pads a 1- or 2-part core, so `0.8` is `0.8.0`."""
    if (version := _parse_semver(raw)) is not None:
        return version
    core = re.split(r"[-+]", raw, maxsplit=1)[0]
    parts = core.split(".")
    if not core or len(parts) >= _CORE_PARTS:
        return None
    padded = ".".join(parts + ["0"] * (_CORE_PARTS - len(parts)))
    return _parse_semver(padded + raw.removeprefix(core))


def planner_keeps_installed(installed: str, served: str) -> bool:
    """Whether the planner refuses to move from `installed` to the lower `served`.

    An unparsable version on either side disables that guard.
    """
    installed_version = parse_package_version(installed)
    served_version = parse_package_version(served)
    if installed_version is None or served_version is None:
        return False
    return served_version < installed_version
