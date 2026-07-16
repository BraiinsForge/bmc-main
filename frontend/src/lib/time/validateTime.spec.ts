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

import { describe, test, expect } from '@rstest/core';

import { TimeFormat } from '@/proto/gen/web/shared_pb';
import { validateTime24, validateTime12, validateTime } from './validateTime';
import { time24to12, time12to24, formatAlarmTime, parseAlarmTime } from './index';

// ─── validateTime24 ──────────────────────────────────────────────

describe('validateTime24', () => {
    test('accepts valid 24h times with two-digit hours', () => {
        expect(validateTime24('00:00')).toBe(true);
        expect(validateTime24('09:30')).toBe(true);
        expect(validateTime24('12:00')).toBe(true);
        expect(validateTime24('23:59')).toBe(true);
    });

    test('accepts single-digit hours', () => {
        expect(validateTime24('0:00')).toBe(true);
        expect(validateTime24('9:30')).toBe(true);
    });

    test('rejects hours out of range', () => {
        expect(validateTime24('24:00')).toBe(false);
        expect(validateTime24('25:00')).toBe(false);
        expect(validateTime24('99:00')).toBe(false);
    });

    test('rejects minutes out of range', () => {
        expect(validateTime24('12:60')).toBe(false);
        expect(validateTime24('12:99')).toBe(false);
    });

    test('rejects missing minutes', () => {
        expect(validateTime24('12')).toBe(false);
        expect(validateTime24('12:')).toBe(false);
    });

    test('rejects single-digit minutes', () => {
        expect(validateTime24('12:5')).toBe(false);
    });

    test('rejects three-digit hours', () => {
        expect(validateTime24('123:00')).toBe(false);
    });

    test('rejects empty and garbage strings', () => {
        expect(validateTime24('')).toBe(false);
        expect(validateTime24('abc')).toBe(false);
        expect(validateTime24('12:00 AM')).toBe(false);
        expect(validateTime24(' 12:00')).toBe(false);
        expect(validateTime24('12:00 ')).toBe(false);
    });
});

// ─── validateTime12 ──────────────────────────────────────────────

describe('validateTime12', () => {
    test('accepts valid 12h times', () => {
        expect(validateTime12('1:00')).toBe(true);
        expect(validateTime12('12:00')).toBe(true);
        expect(validateTime12('12:59')).toBe(true);
        expect(validateTime12('1:30 AM')).toBe(true);
        expect(validateTime12('12:00 PM')).toBe(true);
    });

    test('accepts two-digit hours', () => {
        expect(validateTime12('01:00')).toBe(true);
        expect(validateTime12('09:45')).toBe(true);
    });

    test('accepts case-insensitive AM/PM suffix', () => {
        expect(validateTime12('1:00 am')).toBe(true);
        expect(validateTime12('1:00 Pm')).toBe(true);
    });

    test('rejects hour 0 (invalid in 12h format)', () => {
        expect(validateTime12('0:00')).toBe(false);
        expect(validateTime12('00:00')).toBe(false);
    });

    test('rejects hours above 12', () => {
        expect(validateTime12('13:00')).toBe(false);
        expect(validateTime12('23:00')).toBe(false);
    });

    test('rejects minutes out of range', () => {
        expect(validateTime12('12:60')).toBe(false);
        expect(validateTime12('12:99')).toBe(false);
    });

    test('rejects empty and garbage strings', () => {
        expect(validateTime12('')).toBe(false);
        expect(validateTime12('abc')).toBe(false);
        expect(validateTime12('AM')).toBe(false);
    });
});

// ─── validateTime (unified) ─────────────────────────────────────

describe('validateTime', () => {
    test('delegates to 24h validator', () => {
        expect(validateTime('23:59', TimeFormat.TIME_FORMAT_24_HOUR)).toBe(true);
        expect(validateTime('0:00', TimeFormat.TIME_FORMAT_24_HOUR)).toBe(true);
        expect(validateTime('13:00 PM', TimeFormat.TIME_FORMAT_24_HOUR)).toBe(false);
    });

    test('delegates to 12h validator', () => {
        expect(validateTime('12:30 AM', TimeFormat.TIME_FORMAT_12_HOUR)).toBe(true);
        expect(validateTime('0:00', TimeFormat.TIME_FORMAT_12_HOUR)).toBe(false);
    });
});

// ─── time24to12 ──────────────────────────────────────────────────

describe('time24to12', () => {
    test('converts midnight', () => {
        expect(time24to12('00:00')).toBe('12:00 AM');
    });

    test('converts noon', () => {
        expect(time24to12('12:00')).toBe('12:00 PM');
    });

    test('converts morning hours', () => {
        expect(time24to12('01:30')).toBe('1:30 AM');
        expect(time24to12('09:05')).toBe('9:05 AM');
        expect(time24to12('11:59')).toBe('11:59 AM');
    });

    test('converts afternoon/evening hours', () => {
        expect(time24to12('13:00')).toBe('1:00 PM');
        expect(time24to12('18:45')).toBe('6:45 PM');
        expect(time24to12('23:59')).toBe('11:59 PM');
    });

    test('returns input unchanged for non-matching strings', () => {
        expect(time24to12('abc')).toBe('abc');
        expect(time24to12('')).toBe('');
    });
});

// ─── time12to24 ──────────────────────────────────────────────────

describe('time12to24', () => {
    test('converts 12 AM to 00', () => {
        expect(time12to24('12:00 AM')).toBe('00:00');
    });

    test('converts 12 PM to 12', () => {
        expect(time12to24('12:00 PM')).toBe('12:00');
    });

    test('converts AM hours', () => {
        expect(time12to24('1:30 AM')).toBe('01:30');
        expect(time12to24('9:05 AM')).toBe('09:05');
        expect(time12to24('11:59 AM')).toBe('11:59');
    });

    test('converts PM hours', () => {
        expect(time12to24('1:00 PM')).toBe('13:00');
        expect(time12to24('6:45 PM')).toBe('18:45');
        expect(time12to24('11:59 PM')).toBe('23:59');
    });

    test('is case-insensitive', () => {
        expect(time12to24('1:00 am')).toBe('01:00');
        expect(time12to24('1:00 pm')).toBe('13:00');
    });

    test('returns input unchanged for non-matching strings', () => {
        expect(time12to24('abc')).toBe('abc');
        expect(time12to24('13:00')).toBe('13:00');
    });
});

// ─── round-trip: 24→12→24 ───────────────────────────────────────

describe('time24to12 / time12to24 round-trip', () => {
    const cases = ['00:00', '01:30', '09:05', '12:00', '13:00', '18:45', '23:59'];

    for (const t of cases) {
        test(`round-trips ${t}`, () => {
            expect(time12to24(time24to12(t))).toBe(t);
        });
    }
});

// ─── formatAlarmTime / parseAlarmTime ────────────────────────────

describe('formatAlarmTime', () => {
    test('returns 12h format when configured', () => {
        expect(formatAlarmTime('13:00', TimeFormat.TIME_FORMAT_12_HOUR)).toBe('1:00 PM');
    });

    test('returns 24h format unchanged when configured', () => {
        expect(formatAlarmTime('13:00', TimeFormat.TIME_FORMAT_24_HOUR)).toBe('13:00');
    });
});

describe('parseAlarmTime', () => {
    test('converts 12h input to 24h', () => {
        expect(parseAlarmTime('1:00 PM', TimeFormat.TIME_FORMAT_12_HOUR)).toBe('13:00');
    });

    test('passes through 24h input', () => {
        expect(parseAlarmTime('13:00', TimeFormat.TIME_FORMAT_24_HOUR)).toBe('13:00');
    });
});
