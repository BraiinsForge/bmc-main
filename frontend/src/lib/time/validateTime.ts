import { TimeFormat } from '@/proto/gen/web/shared_pb';

/** Parse `H:MM` or `HH:MM` and check that hours/minutes are within the given ranges. */
function isValidTime(time: string, minHour: number, maxHour: number): boolean {
    const match = time.match(/^(\d{1,2}):(\d{2})$/);
    if (!match) return false;

    const hours = Number(match[1]);
    const minutes = Number(match[2]);

    return hours >= minHour && hours <= maxHour && minutes >= 0 && minutes <= 59;
}

/** Validate a time string in the format `H:MM` or `HH:MM` with hours 0–23. */
export function validateTime24(time: string): boolean {
    return isValidTime(time, 0, 23);
}

/** Validate a time string in the format `H:MM` or `HH:MM` with hours 1–12. Ignores a trailing AM/PM suffix. */
export function validateTime12(time: string): boolean {
    return isValidTime(time.replace(/\s*(AM|PM)\s*$/i, ''), 1, 12);
}

/** Validate a time string according to the given time format. */
export function validateTime(time: string, format: TimeFormat): boolean {
    return format === TimeFormat.TIME_FORMAT_12_HOUR ? validateTime12(time) : validateTime24(time);
}
