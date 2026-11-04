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

import { describe, expect, test } from '@rstest/core';
import { createIntl } from 'react-intl';

import * as pb from '@/proto';

const { MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY, SUNDAY } = pb.Weekday;
const intl = createIntl({ locale: 'en' });

describe('weekdaysFrom', () => {
    test.each([
        ['Monday', MONDAY, [MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY, SUNDAY]],
        ['Sunday', SUNDAY, [SUNDAY, MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY]],
        ['Wednesday', WEDNESDAY, [WEDNESDAY, THURSDAY, FRIDAY, SATURDAY, SUNDAY, MONDAY, TUESDAY]],
    ])('a week starting on %s', (_case, first, expected) => {
        expect(pb.weekdaysFrom(first)).toEqual(expected);
    });

    test('an unspecified first day leaves the week starting on Monday', () => {
        expect(pb.weekdaysFrom(pb.Weekday.UNSPECIFIED)[0]).toBe(MONDAY);
    });
});

describe('weekdayListToString', () => {
    test.each([
        [MONDAY, 'Mon, Wed, Sun'],
        [SUNDAY, 'Sun, Mon, Wed'],
        [WEDNESDAY, 'Wed, Sun, Mon'],
    ])('a list under first day %i reads %s', (first, expected) => {
        expect(pb.weekdayListToString(intl, [SUNDAY, MONDAY, WEDNESDAY], first)).toBe(expected);
    });

    test.each([
        ['All days', [SUNDAY, MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY]],
        ['Weekdays', [MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY]],
        ['Weekends', [SUNDAY, SATURDAY]],
    ])('%s names the same days under any first day', (expected, days) => {
        for (const first of [MONDAY, WEDNESDAY, SUNDAY]) {
            expect(pb.weekdayListToString(intl, days, first)).toBe(expected);
        }
    });
});
