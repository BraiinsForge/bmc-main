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

import type { Capabilities } from '@/lib/system';

/** Deck hardware capabilities for specs; override only what the test is about. */
export function deckCapabilities(overrides: Partial<Capabilities> = {}): Capabilities {
    return {
        combinedScenesSupported: true,
        wifiSupported: true,
        ethernetSupported: false,
        miningSupported: false,
        soundSupported: true,
        ledSupported: true,
        alarmSupported: true,
        boserManaged: false,
        productName: 'Braiins Deck',
        ...overrides,
    };
}

/** `window.SYSTEM` as system.js assigns it, before `parseSystem`: a BMC 100 unless overridden. */
export function deckSystemWire(overrides: Record<string, unknown> = {}) {
    return {
        capabilities: {
            display: { width: 1280, height: 480, shape: 'Rectangular', dpi: 217 },
            slot_grid: { columns: 4, rows: 2 },
            wifi_supported: true,
            ethernet_supported: false,
            mining_supported: false,
            sound_supported: true,
            led_supported: true,
            alarm_supported: true,
            boser_managed: false,
            product_name: 'Braiins Deck',
            ...overrides,
        },
    };
}
