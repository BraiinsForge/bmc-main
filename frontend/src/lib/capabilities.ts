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

import type { Capabilities } from '@/lib/system';

export function ethernetConfigurable(caps: Capabilities): boolean {
    return caps.ethernetSupported && !caps.boserManaged;
}

export function wifiConfigurable(caps: Capabilities): boolean {
    return caps.wifiSupported && !caps.boserManaged;
}

export function networkConfigurable(caps: Capabilities): boolean {
    return ethernetConfigurable(caps) || wifiConfigurable(caps);
}

export function soundConfigurable(caps: Capabilities): boolean {
    return caps.soundSupported;
}

export function ledConfigurable(caps: Capabilities): boolean {
    return caps.ledSupported;
}

export function soundOrLightConfigurable(caps: Capabilities): boolean {
    return soundConfigurable(caps) || ledConfigurable(caps);
}

// The backend derives alarm support from its outputs; a second rule here could drift from it.
export function alarmsAvailable(caps: Capabilities): boolean {
    return caps.alarmSupported;
}

function boserManaged(caps: Capabilities): boolean {
    return caps.boserManaged;
}

// What boser's own UI covers on a managed device. All track `boserManaged` today;
// BMC and boser upgrade independently, so the split is kept per feature.
export function securityManaged(caps: Capabilities): boolean {
    return boserManaged(caps);
}

export function upgradesManaged(caps: Capabilities): boolean {
    return boserManaged(caps);
}

export function timezoneConfigurable(caps: Capabilities): boolean {
    return !boserManaged(caps);
}

export function systemActionsOwned(caps: Capabilities): boolean {
    return boserManaged(caps);
}

export function loginOwned(caps: Capabilities): boolean {
    return boserManaged(caps);
}

export function boserChrome(caps: Capabilities): boolean {
    return boserManaged(caps);
}
