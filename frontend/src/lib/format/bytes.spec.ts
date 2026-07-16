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

import { describe, test, expect } from '@rstest/core';

import { formatBytes } from './bytes';

describe('formatBytes', () => {
    test('formats sub-kB counts with a plain B unit', () => {
        expect(formatBytes(0)).toBe('0 B');
        expect(formatBytes(1)).toBe('1 B');
        expect(formatBytes(500)).toBe('500 B');
        expect(formatBytes(999)).toBe('999 B');
    });

    test('applies SI prefixes on a decimal scale (1 kB = 1000 B)', () => {
        expect(formatBytes(1_000)).toBe('1 kB');
        expect(formatBytes(1_500)).toBe('1.5 kB');
        expect(formatBytes(348_000_000)).toBe('348 MB');
        expect(formatBytes(2_000_000_000)).toBe('2 GB');
    });

    test('keeps up to 4 significant figures, trimming trailing zeros', () => {
        expect(formatBytes(1_234_567)).toBe('1.235 MB');
    });

    test('accepts bigint, the uint64 wire type for byte fields', () => {
        expect(formatBytes(365_000_000n)).toBe('365 MB');
        expect(formatBytes(1_500n)).toBe('1.5 kB');
    });

    test('preserves the sign of a negative count', () => {
        // d3-format renders the sign with U+2212 (MINUS SIGN), not an ASCII hyphen.
        expect(formatBytes(-1_500)).toBe('−1.5 kB');
        expect(formatBytes(-500)).toBe('−500 B');
    });

    test('returns the placeholder for non-finite input', () => {
        expect(formatBytes(Number.NaN)).toBe('—');
        expect(formatBytes(Number.POSITIVE_INFINITY, 'n/a')).toBe('n/a');
    });
});
