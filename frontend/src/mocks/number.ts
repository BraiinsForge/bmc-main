// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { random, randomInt } from 'es-toolkit';

export function number(min: number, max: number, float: boolean | number = false): number {
    const v = !float ? randomInt(min, max) : random(min, max);
    return typeof float === 'number' ? Number.parseFloat(v.toFixed(float)) : v;
}

export const binary = (byteLength: number, signed: boolean = false): number => {
    const r = binary.range(byteLength, signed);
    return number(r[0], r[1], false);
};
binary.range = (byteLength: number, signed: boolean = false): [number, number] => {
    const nbits = byteLength * 8;

    // * Unsigned:             0  ->  2^n - 1
    // *   Signed:    -2^(n - 1)  ->  2^(n - 1) - 1
    const min = signed ? (-2) ** (nbits - 1) : 0;
    const max = 2 ** (nbits - Number(signed)) - 1;

    return [min, max];
};

export function semver(statics?: { major: number; minor: number; patch: number }): string {
    const major = statics?.major ?? number(0, 10);
    const minor = statics?.minor ?? number(0, 10);
    const patch = statics?.patch ?? number(0, 10);
    return [major, minor, patch].filter(Boolean).join('.');
}
