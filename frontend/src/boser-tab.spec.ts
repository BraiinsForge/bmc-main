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

import fs from 'node:fs';
import path from 'node:path';

import { afterEach, describe, expect, test } from '@rstest/core';

import { parseBrand } from '@/lib/brand';
import { boserChrome } from '@/lib/capabilities';
import { parseSystem } from '@/lib/system';
import { deckSystemWire } from '@/pages/workspace/Display/capabilities.fixture';

const SCRIPT = fs.readFileSync(path.join(__dirname, 'boser-tab.js'), 'utf8');
const DECK_TITLE = 'Braiins DECK';
const DECK_ICON = '/icon.png';

type Globals = { SYSTEM?: unknown; BRAND?: unknown };

function runScript(globals: Globals): { title: string; icon: null | string } {
    document.head.innerHTML = `<link rel="icon" href="${DECK_ICON}">`;
    document.title = DECK_TITLE;
    Object.assign(window, globals);
    new Function(SCRIPT)();
    return { title: document.title, icon: document.querySelector('link[rel="icon"]')?.getAttribute('href') ?? null };
}

afterEach(() => {
    const w = window as Globals;
    delete w.SYSTEM;
    delete w.BRAND;
});

const BRANDS: Array<[string, unknown]> = [
    ['a named brand', { name: 'Acme OS' }],
    ['a brand with an empty name', { name: '' }],
    ['a brand whose name is not a string', { name: 42 }],
    ['no brand', undefined],
];

describe('boser-tab.js', () => {
    for (const boserManaged of [true, false]) {
        for (const [label, raw] of BRANDS) {
            test(`${boserManaged ? 'a managed' : 'a standalone'} device with ${label} agrees with the app`, () => {
                const system = deckSystemWire({ boser_managed: boserManaged });

                const tab = runScript({ SYSTEM: system, BRAND: raw });

                const branded = boserChrome(parseSystem(system).capabilities) && raw !== undefined;
                const name = branded ? parseBrand(raw).name : null;
                expect(tab.title).toBe(name ?? DECK_TITLE);
                expect(tab.icon).toMatch(branded ? /^\/var\/favicon\.png\?v=\d+$/ : new RegExp(`^${DECK_ICON}$`));
            });
        }
    }

    test('does nothing before system.js ran', () => {
        expect(runScript({})).toEqual({ title: DECK_TITLE, icon: DECK_ICON });
    });
});
