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
