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

import { describe, expect, test } from '@rstest/core';

import { deckSystemWire } from '@/pages/workspace/Display/capabilities.fixture';
import { parseSystem } from './system';

const bmc100 = deckSystemWire();

describe('parseSystem', () => {
    test('maps the platform fields', () => {
        expect(parseSystem(bmc100)).toEqual({
            capabilities: {
                combinedScenesSupported: true,
                wifiSupported: true,
                ethernetSupported: false,
                miningSupported: false,
                soundSupported: true,
                ledSupported: true,
                alarmSupported: true,
                boserManaged: false,
                productName: 'Braiins Deck',
            },
        });
    });

    test('derives combined scenes from the slot grid', () => {
        const { capabilities } = parseSystem(deckSystemWire({ slot_grid: null }));

        expect(capabilities.combinedScenesSupported).toBe(false);
    });

    test('throws when the global is missing', () => {
        expect(() => parseSystem(undefined)).toThrow('window.SYSTEM');
    });

    test('throws on a flag of the wrong type', () => {
        const raw = deckSystemWire({ boser_managed: 'yes' });

        expect(() => parseSystem(raw)).toThrow('`capabilities.boser_managed`');
    });

    test('names the capabilities object when it is missing', () => {
        expect(() => parseSystem({})).toThrow('`capabilities` is missing');
    });
});
