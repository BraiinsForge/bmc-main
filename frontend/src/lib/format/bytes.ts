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

import { format } from 'd3-format';

// SI-prefixed, up to 4 significant figures, trailing zeros trimmed.
// Decimal scale (1 kB = 1000 B).
const fmt = format('.4~s');

/**
 * Format a byte count as a human-readable string with an SI unit.
 *
 * `1_500n` → `"1.5 kB"`, `348_000_000` → `"348 MB"`, `500` → `"500 B"`.
 * Accepts `bigint` (the wire type for `uint64` byte fields) or `number`;
 * non-finite input returns `placeholder`.
 */
export function formatBytes(bytes: bigint | number, placeholder = '—'): string {
    const n = Number(bytes);
    if (!Number.isFinite(n)) return placeholder;

    // d3 SI output is a (possibly signed) number ending in at most one prefix
    // letter (k/M/G/…). Split off that letter and keep everything else verbatim,
    // so the sign (d3 uses U+2212) and decimals survive.
    const formatted = fmt(n);
    const prefix = /\p{L}$/u.test(formatted) ? formatted.slice(-1) : '';
    const value = prefix ? formatted.slice(0, -1) : formatted;
    return `${value} ${prefix}B`;
}
