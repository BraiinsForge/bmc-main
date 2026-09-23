// Copyright (C) 2026  Braiins Systems s.r.o.
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

import { onTestFinished } from '@rstest/core';

type Navigations = Partial<Pick<Location, 'assign' | 'replace' | 'reload'>>;

/**
 * jsdom won't let a spec spy on `location.assign` and friends, so this swaps the whole location
 * for the current URL carrying the given spies, and puts the real one back once the test finishes.
 *
 * Kept out of the `@/mocks` barrel: stories import that, and this pulls in the test runner.
 */
export function stubLocation(navigations: Navigations): void {
    const real = window.location;
    onTestFinished(() => {
        Object.defineProperty(window, 'location', { configurable: true, value: real });
    });
    Object.defineProperty(window, 'location', {
        configurable: true,
        value: Object.assign(new URL(real.href), navigations),
    });
}
