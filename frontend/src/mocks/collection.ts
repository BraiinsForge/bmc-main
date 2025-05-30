import { random } from 'es-toolkit';
import { knuthShuffle } from 'knuth-shuffle';
import invariant from 'invariant';

import COLORS from '@/styles/colors';
import { number } from './number';
import { type LengthRange, getLength } from './generics';

export function randomItem<T>(input: ReadonlyArray<T>): T {
    return input[random(0, input.length - 1)];
}
export function randomEnumItem<V extends string | number>(enm: Record<string, V>): V {
    const input = Object.values(enm) as V[];
    return randomItem<V>(input);
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

export function recordOf<K extends Key = StrNum, V = unknown>(keys: K[], value: (key: K) => V): Record<K, V> {
    const res = {} as Record<K, V>;
    keys.forEach(k => (res[k] = value(k)));
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
