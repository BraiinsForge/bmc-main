// Copyright (C) 2026  Braiins Systems s.r.o.
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
import { createIntl } from 'react-intl';

import { formatDuration } from './duration';

const en = createIntl({ locale: 'en' });

describe('formatDuration', () => {
    test('names both parts when neither is zero', () => {
        expect(formatDuration(en, 90)).toBe('1 minute, 30 seconds');
    });

    test('leaves a zero part out', () => {
        expect(formatDuration(en, 45)).toBe('45 seconds');
        expect(formatDuration(en, 120)).toBe('2 minutes');
    });

    test('names zero time in seconds', () => {
        expect(formatDuration(en, 0)).toBe('0 seconds');
    });

    test('drops a fraction of a second', () => {
        expect(formatDuration(en, 61.9)).toBe('1 minute, 1 second');
    });

    test("words the units in the locale's own language", () => {
        expect(formatDuration(createIntl({ locale: 'zh' }), 90)).toBe('1分钟30秒钟');
    });
});
