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
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import {
    alarmsAvailable,
    boserChrome,
    ethernetConfigurable,
    networkConfigurable,
    soundOrLightConfigurable,
    wifiConfigurable,
} from './capabilities';

describe('capability predicates', () => {
    test('a standalone Deck configures the interfaces it has', () => {
        const caps = deckCapabilities({ wifiSupported: true, ethernetSupported: false, boserManaged: false });

        expect(wifiConfigurable(caps)).toBe(true);
        expect(ethernetConfigurable(caps)).toBe(false);
        expect(networkConfigurable(caps)).toBe(true);
    });

    test('a boser-managed device leaves networking to boser', () => {
        const caps = deckCapabilities({ wifiSupported: true, ethernetSupported: true, boserManaged: true });

        expect(networkConfigurable(caps)).toBe(false);
        expect(boserChrome(caps)).toBe(true);
    });

    test('either output alone opens Sound & Light', () => {
        expect(soundOrLightConfigurable(deckCapabilities({ soundSupported: true, ledSupported: false }))).toBe(true);
        expect(soundOrLightConfigurable(deckCapabilities({ soundSupported: false, ledSupported: true }))).toBe(true);
    });

    test('no output closes Sound & Light', () => {
        expect(soundOrLightConfigurable(deckCapabilities({ soundSupported: false, ledSupported: false }))).toBe(false);
    });

    test('alarm availability is the backend flag, not a local sound-or-LED rule', () => {
        const caps = deckCapabilities({ soundSupported: true, ledSupported: true, alarmSupported: false });

        expect(alarmsAvailable(caps)).toBe(false);
    });
});
