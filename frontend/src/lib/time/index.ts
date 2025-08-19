import { timeFormat } from 'd3-time-format';
export * from './tz';

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
export function toTimestamp<T extends Date | Timestamp | bigint>(x: T | Date): Timestamp {
    switch (true) {
        case typeof x === 'bigint':
            // Seconds, so just casting needs to be done
            return Number(x);

        case x instanceof Date:
            // Date gives us miliseconds when cas to number,
            // so we need to convert & round
            return Math.floor(Number(x) / 1e3);

        default:
            return x as Timestamp;
    }
}

/** Convert timestamp to local with info from global tzname variable */
export const localTimeFormat = (
    value: Date | Timestamp | bigint,
    format?: null | string,
    tzname?: Maybe<string>,
): string => {
    const dateUTC = new Date(toTimestamp(value) * 1e3);

    // Timezone formatting can throw is the TZ name is unrecognized
    try {
        const dateTZ = new Date(dateUTC.toLocaleString('en-US', { timeZone: tzname || 'UTC' }));
        return timeFormat(format || '%d.%m.%Y %H:%M')(dateTZ);
    } catch {
        return 'INVALID TIME(ZONE)';
    }
};
