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

import { describe, expect, test } from '@rstest/core';

import { renamedAddress } from './renamedAddress';

describe('renamedAddress', () => {
    // The old name stops resolving after the rename, so staying on it strands the page.
    test('a page opened under the old .local name moves to the new one', () => {
        expect(renamedAddress('braiins-deck.local', 'braiins-deck', 'kitchen-deck', '10.0.0.7')).toBe(
            'kitchen-deck.local',
        );
    });

    test('browsers lowercase the host, so a mixed-case hostname still matches', () => {
        expect(renamedAddress('braiins-deck.local', 'Braiins-Deck', 'kitchen', '10.0.0.7')).toBe('kitchen.local');
    });

    // An IP address keeps working across the rename; moving would drop the session for nothing.
    test('a page opened by IP address stays where it is', () => {
        expect(renamedAddress('192.168.1.20', 'braiins-deck', 'kitchen-deck', '192.168.1.20')).toBeNull();
    });

    // A dotted hostname is announced under an altered name, so the typed one would not resolve.
    test('a name the device does not announce as typed sends the page to the IP address', () => {
        expect(renamedAddress('braiins-deck.local', 'braiins-deck', 'kitchen.deck', '10.0.0.7')).toBe('10.0.0.7');
    });
});
