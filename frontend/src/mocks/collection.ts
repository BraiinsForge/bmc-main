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

import invariant from 'invariant';
import { random } from 'es-toolkit';
import { knuthShuffle } from 'knuth-shuffle';

import COLORS from '@/styles/colors';
import { number } from './number';
import { type LengthRange, getLength } from './generics';

export function randomItem<T>(input: ReadonlyArray<T>): T {
    return input[random(0, input.length - 1)];
}

export function randomSlices(
    pool: number, // space to split
    count: number, // number of slices
    min: number = 0, // minimum slice size (this rule can be broken towards the end)
    max: number = pool * 0.7, // maximum value slice size (this rule can be broken towards the end)
    nice: (n: number) => number = Math.floor, // Format the slice value to something nice
): number[] {
    invariant(typeof count === 'number' && count < pool, `invalid "count = ${count}" (must be smaller than "${pool}")`);
    invariant(typeof max === 'number' && max <= pool, `invalid "max = ${max}". Must be smaller than "${pool}"`);
    invariant(typeof nice === 'function', 'invalid "nice" argument');

    const res: number[] = [];

    let rest = pool;
    let i = count + 1;
    while (--i > 0) {
        const isLast = i === 1;
        const isEmpty = min ? rest <= min : rest === 0;

        if (isLast || isEmpty) {
            res.push(rest);
        } else {
            const candidate = nice(number(min, Math.min(max, rest)));
            const value = candidate < min || rest - candidate < min ? rest : candidate;

            res.push(value);
            rest -= value;
        }
    }

    return res;
}

export function arrayOf<T = number>(length: LengthRange, value?: MaybeGetter<T, [number]>): T[] {
    return Array.from({ length: getLength(length) }, (_, index) => {
        const v = typeof value === 'function' ? (value as Getter<T, [number]>)(index) : value;
        return v as T;
    });
}

export function recordOf<K extends PropertyKey = StrNum, V = unknown>(keys: K[], value: (key: K) => V): Record<K, V> {
    const res = {} as Record<K, V>;
    keys.forEach(k => {
        res[k] = value(k);
    });
    return res;
}

/** Get random slice of the given array */
export function randomSlice<T>(pool: ReadonlyArray<T>, count: LengthRange): T[] {
    return knuthShuffle(pool.slice(0)).slice(0, Math.min(pool.length, getLength(count)));
}

export const color = (): string => randomItem(Object.values(COLORS));
export function randColor(alpha: number = 1): string {
    const cols = [number(0, 250, false), number(0, 250, false), number(0, 250, false), alpha];
    return `rgba(${cols.join(' ,')})`;
}
export function randColors(count: number, alpha: number = 1): string[] {
    return arrayOf<string>(count, () => randColor(alpha));
}
