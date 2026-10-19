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

"""Typed stand-ins for `monkeypatch.setattr`, which types the replacement only as `object`."""

from collections.abc import Callable
from typing import NoReturn


def returning[T](value: T) -> Callable[..., T]:
    """A stand-in that ignores its arguments and returns `value`."""

    def stub(*_args: object, **_kwargs: object) -> T:
        return value

    return stub


def recording(calls: list[str], name: str) -> Callable[..., None]:
    """A stand-in that ignores its arguments and records `name` in `calls`."""

    def stub(*_args: object, **_kwargs: object) -> None:
        calls.append(name)

    return stub


def raising(error: BaseException) -> Callable[..., NoReturn]:
    """A stand-in that ignores its arguments and raises `error`."""

    def stub(*_args: object, **_kwargs: object) -> NoReturn:
        raise error

    return stub
