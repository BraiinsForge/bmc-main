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

"""Structure tests for the fault-injection suite."""

from bmc_tui.procedures import e2e_sysupgrade_faults as faults


def test_suite_order_is_pinned() -> None:
    assert faults.SUITE_ORDER == (
        "unsigned-feed",
        "untrusted-key-name",
        "wrong-key-signature",
        "corrupt-tarball",
        "download-stall",
        "good-init",
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "blank-data-partition",
        "corrupt-fs-metadata",
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "cache-swap-retry",
        "same-version-reflash",
    )


def test_every_scenario_id_has_a_driver() -> None:
    ids = {
        "wrong-key-signature",
        "unsigned-feed",
        "untrusted-key-name",
        "corrupt-tarball",
        "download-stall",
        "blank-data-partition",
        "corrupt-fs-metadata",
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "cache-swap-retry",
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "stale-next-marker",
        "same-version-reflash",
        "shm-local-file",
        "staged-once",
        "servers-json",
    }
    groups = {"a", "b", "c", "d", "all"}
    assert ids | groups <= set(faults._DRIVERS)


def test_suite_order_ids_are_driven() -> None:
    assert all(sid in faults._DRIVERS or sid == "good-init" for sid in faults.SUITE_ORDER)
