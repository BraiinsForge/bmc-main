import { describe, test, expect } from 'vitest';
import * as fc from 'fast-check';

import { randomSlices } from './index';

describe('mocks/generic', () => {
    describe('randomSlices', () => {
        test('returns correct number of items', () => {
            fc.assert(
                fc.property(fc.integer({ min: 1, max: 99 }), num => {
                    expect(randomSlices(100, num)).toHaveLength(num);
                }),
            );
        });

        test('obeys the lower value boundary', () => {
            fc.assert(
                fc.property(fc.integer({ min: 1, max: 30 }), limit => {
                    const arr = randomSlices(100, 10, limit);
                    const offenders = arr.filter(x => x !== 0 && x < limit);
                    expect(offenders).toHaveLength(0);
                }),
                { numRuns: 100 },
            );
        });

        test('obeys the upper value boundary', () => {
            fc.assert(
                fc.property(fc.double({ min: 0.3, max: 0.7, noDefaultInfinity: true, noNaN: true }), limit => {
                    const pool = 100;
                    const max = 100 * limit;
                    const arr = randomSlices(pool, 10, limit, max);
                    const offenders = arr
                        .filter(Boolean)
                        .slice(0, -1)
                        .filter(x => x > max);
                    expect(offenders).toHaveLength(0);
                }),
                { numRuns: 100 },
            );
        });

        test('results sum equals to given pool size', () => {
            fc.assert(
                fc.property(fc.integer({ min: 100, max: 1000 }), n => {
                    const arr = randomSlices(n, Math.floor(n / 10));
                    expect(arr.reduce((acc, v) => acc + v, 0)).toEqual(n);
                }),
                { numRuns: 100 },
            );
        });
    });
});
