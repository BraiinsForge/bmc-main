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
