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

import { isPlainObject } from 'es-toolkit';

// BMC serves `/system.js`, a script assigning `window.SYSTEM` with the platform's `HardwareCapabilities`.
// BMC always generates the file, so a missing or malformed global is a fault, never a plain Deck.

export interface System {
    capabilities: Capabilities;
}
export interface Capabilities {
    combinedScenesSupported: boolean;
    wifiSupported: boolean;
    ethernetSupported: boolean;
    miningSupported: boolean;
    soundSupported: boolean;
    ledSupported: boolean;
    alarmSupported: boolean;
    boserManaged: boolean;
    productName: string;
}

class SystemParseError extends Error {
    constructor(...path: string[]) {
        super(`system.js: \`${path.join('.')}\` is missing or has the wrong type`);
    }
}

function bool(caps: Record<string, unknown>, key: string): boolean {
    const v = caps[key];
    if (typeof v !== 'boolean') throw new SystemParseError('capabilities', key);
    return v;
}

function parseCapabilities(caps: unknown): Capabilities {
    if (!isPlainObject(caps)) throw new SystemParseError('capabilities');
    const productName = caps.product_name;
    if (typeof productName !== 'string') throw new SystemParseError('capabilities', 'product_name');
    if (caps.slot_grid != null && !isPlainObject(caps.slot_grid))
        throw new SystemParseError('capabilities', 'slot_grid');
    return {
        combinedScenesSupported: caps.slot_grid != null,
        wifiSupported: bool(caps, 'wifi_supported'),
        ethernetSupported: bool(caps, 'ethernet_supported'),
        miningSupported: bool(caps, 'mining_supported'),
        soundSupported: bool(caps, 'sound_supported'),
        ledSupported: bool(caps, 'led_supported'),
        alarmSupported: bool(caps, 'alarm_supported'),
        boserManaged: bool(caps, 'boser_managed'),
        productName,
    };
}

export function parseSystem(raw: unknown): System {
    if (!isPlainObject(raw)) throw new Error('system.js: `window.SYSTEM` is not set');
    return { capabilities: parseCapabilities(raw.capabilities) };
}

/** Reads the global the blocking `system.js` script in index.html assigns before the bundle runs. */
export function readSystem(): System {
    return parseSystem((window as { SYSTEM?: unknown }).SYSTEM);
}
