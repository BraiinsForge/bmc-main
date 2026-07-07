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
