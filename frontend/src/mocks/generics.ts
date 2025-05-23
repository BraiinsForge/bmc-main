import { randomInt } from 'es-toolkit';

export type LengthRange = Integer | [min: number, max: number];
export function getLength(l: LengthRange): Integer {
    if (typeof l === 'number') return l;
    return randomInt(l[0], l[1]);
}

export const boolean = (chance: number = 0.5): boolean => !(Math.random() < chance);
