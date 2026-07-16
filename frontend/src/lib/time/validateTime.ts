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
