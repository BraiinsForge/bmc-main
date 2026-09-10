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

import { afterEach, describe, expect, test } from '@rstest/core';

import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';

afterEach(() => {
    store.setHardwareCapabilities(null);
});

describe('store.state', () => {
    test('throws before boot() loaded the capabilities', () => {
        expect(() => store.state).toThrow('BUG');
    });

    test('is the same object until something changes', () => {
        store.setHardwareCapabilities(deckCapabilities());

        expect(store.state).toBe(store.state);
    });

    test('is a new object after a change', () => {
        store.setHardwareCapabilities(deckCapabilities());
        const before = store.state;

        store.setHardwareCapabilities(deckCapabilities());

        expect(store.state).not.toBe(before);
    });

    test('is frozen', () => {
        store.setHardwareCapabilities(deckCapabilities());

        expect(Object.isFrozen(store.state)).toBe(true);
    });
});
