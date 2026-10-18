// Copyright (C) 2025  Braiins Systems s.r.o.
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

const holds = new Set<symbol>();

function listener(e: BeforeUnloadEvent) {
    // For legacy browsers
    e.returnValue = true;
    e.preventDefault();
}

/** Asks before the page unloads until the returned release runs; releasing twice is harmless. */
function enable(): () => void {
    const hold = Symbol('unloadGuard');
    holds.add(hold);
    window.addEventListener('beforeunload', listener);
    return () => {
        holds.delete(hold);
        if (holds.size === 0) window.removeEventListener('beforeunload', listener);
    };
}

export const unloadGuard = {
    enable,
    get isEnabled(): boolean {
        return holds.size > 0;
    },
};
