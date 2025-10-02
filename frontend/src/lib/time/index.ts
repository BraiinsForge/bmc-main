import type { Timestamp as PbTimestamp } from '@/proto';
import { isPlainObject } from 'es-toolkit';
import { timeFormat } from 'd3-time-format';

export * from './tz';
export * from './validateTime';

type GetTimestamp = (offset?: null | number, date?: Date | number) => number;
export const getTimestampMs: GetTimestamp = (offset, time): number => {
    const d = time ? new Date(time).getTime() : Date.now();
    const o = offset ?? 0;
    return Math.floor(d + o);
};
export const getTimestamp: GetTimestamp = (offset, time): number => {
    const o = (offset ?? 0) * 1e3; // Convert offset to milliseconds
    return Math.floor(getTimestampMs(o, time) / 1e3);
};

/** Js Date object from unix timestamp (seconds) */
export const toDate = (value: Timestamp | Date): Date => (value instanceof Date ? value : new Date(value * 1e3));

/** Unix timestamp (seconds) from (potentially) js Date object */
export function toTimestamp<T extends Date | Timestamp | bigint | PbTimestamp>(x: T | Date): Timestamp {
    // Date gives us miliseconds when cast to number,
    // so we need to convert & round
    if (x instanceof Date) return Math.floor(Number(x) / 1e3);

    // Sance number
    if (typeof x === 'number' && Number.isFinite(x)) return Math.floor(x);

    // Seconds in bigint
    if (typeof x === 'bigint') return Number(x);

    // Proto timestamp
    if (isPlainObject(x) && 'seconds' in x) return Number(x.seconds);

    throw new TypeError('Invalid time type', { cause: x });
}

/** Convert timestamp to local with info from global tzname variable */
export const localTimeFormat = (
    value: Date | Timestamp | PbTimestamp | bigint,
    format?: null | string,
    tzname?: Maybe<string>,
    fallback: string = 'INVALID TIME(ZONE)',
): string => {
    // Timezone formatting can throw is the TZ name is unrecognized
    try {
        const dateUTC = new Date(toTimestamp(value) * 1e3);
        const dateTZ = new Date(dateUTC.toLocaleString('en-US', { timeZone: tzname || 'UTC' }));
        return timeFormat(format || '%d.%m.%Y %H:%M')(dateTZ);
    } catch {
        return fallback;
    }
};
