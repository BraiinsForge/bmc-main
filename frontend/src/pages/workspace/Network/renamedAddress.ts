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

const MDNS_DOMAIN = '.local';
/** The device announces any other hostname under an altered name the page cannot know. */
const ANNOUNCED_AS_TYPED = /^[A-Za-z0-9-]+$/;

/**
 * A rename withdraws the old `.local` name, so a page opened under it has to
 * move: to the new name when the device announces it as typed, else to the
 * IP address. A page opened by IP address stays where it is.
 */
export function renamedAddress(pageHost: string, previous: string, next: string, ipAddress: string): null | string {
    if (pageHost.toLowerCase() !== `${previous}${MDNS_DOMAIN}`.toLowerCase()) return null;
    if (ANNOUNCED_AS_TYPED.test(next)) return `${next}${MDNS_DOMAIN}`;
    if (ipAddress === '') return null;
    return ipAddress.includes(':') ? `[${ipAddress}]` : ipAddress;
}
