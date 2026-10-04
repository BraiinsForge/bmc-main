// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use super::{Snapshot, SystemSnapshotProvider};

std::thread_local! {
    static NATIVE_SNAPSHOT: core::cell::RefCell<Snapshot> =
        const { core::cell::RefCell::new(Snapshot::empty()) };
}

/// Install the snapshot [`current`](super::current) returns on this thread.
pub fn set_current(snapshot: Snapshot) {
    NATIVE_SNAPSHOT.with(|it| *it.borrow_mut() = snapshot);
}

pub(super) struct Host;

impl SystemSnapshotProvider for Host {
    fn current() -> Snapshot {
        NATIVE_SNAPSHOT.with(|it| it.borrow().clone())
    }

    fn previous() -> Snapshot {
        Snapshot::default()
    }
}
