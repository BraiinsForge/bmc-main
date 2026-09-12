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

use std::cell::RefCell;

use crate::model::Tle;
use crate::orbit::OrbitModel;

struct CachedOrbit {
    tle: Tle,
    model: Option<OrbitModel>,
}

thread_local! {
    static ORBIT_CACHE: RefCell<Option<CachedOrbit>> = const { RefCell::new(None) };
}

pub(crate) fn with_orbit_model<T>(
    tle: &Tle,
    use_model: impl FnOnce(&OrbitModel) -> T,
) -> Option<T> {
    ORBIT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|cached| cached.tle != *tle) {
            *cache = Some(CachedOrbit {
                tle: tle.clone(),
                model: OrbitModel::from_tle(tle),
            });
        }
        cache
            .as_ref()
            .and_then(|cached| cached.model.as_ref())
            .map(use_model)
    })
}

pub(crate) fn has_orbit_model(tle: &Tle) -> bool {
    with_orbit_model(tle, |_| ()).is_some()
}
