// Copyright (C) 2025  Braiins Systems s.r.o.
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

import type { Timestamp as PbTimestamp } from '@/proto';
import { TimeFormat } from '@/proto/gen/web/shared_pb';
import { isPlainObject } from 'es-toolkit';
import { timeFormat } from 'd3-time-format';

export * from './tz';
export * from './validateTime';

/**
 * Convert a 24h time string ("HH:MM") to 12h format ("h:MM AM/PM").
 * Returns the input unchanged if it doesn't match "HH:MM".
 */
export function time24to12(time24: string): string {
    const match = time24.match(/^(\d{1,2}):(\d{2})$/);
    if (!match) return time24;

    const h = Number(match[1]);
    const m = match[2];
    const period = h >= 12 ? 'PM' : 'AM';
    const hour12 = h % 12 || 12;
    return `${hour12}:${m} ${period}`;
}

/**
 * Convert a 12h time string ("h:MM AM/PM") to 24h format ("HH:MM").
 * Returns the input unchanged if it doesn't match 12h format.
 */
export function time12to24(time12: string): string {
    const match = time12.match(/^(\d{1,2}):(\d{2})\s*(AM|PM)$/i);
    if (!match) return time12;

    let hour = Number(match[1]);
    const min = match[2];
    const period = match[3].toUpperCase();

    if (period === 'PM' && hour !== 12) hour += 12;
    if (period === 'AM' && hour === 12) hour = 0;

    return `${String(hour).padStart(2, '0')}:${min}`;
}

/** Format an alarm time string ("HH:MM") according to the given time format setting. */
export function formatAlarmTime(time24: string, format: TimeFormat): string {
    return format === TimeFormat.TIME_FORMAT_12_HOUR ? time24to12(time24) : time24;
}

/** Parse a user-entered alarm time string back to 24h "HH:MM" for the backend. */
export function parseAlarmTime(input: string, format: TimeFormat): string {
    return format === TimeFormat.TIME_FORMAT_12_HOUR ? time12to24(input) : input;
}

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
