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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';

import { bootLocale, intlFor } from '@/lib/intl';

describe('intlFor', () => {
    afterEach(() => {
        rstest.restoreAllMocks();
    });

    test.each(['en', 'en-GB', 'en-US'])('%s speaks the source without reading it as missing', tag => {
        const error = rstest.spyOn(console, 'error').mockImplementation(() => {});

        const saved = intlFor({ tag }).formatMessage({ id: 'save', defaultMessage: 'Save' });

        expect(saved).toBe('Save');
        expect(error).not.toHaveBeenCalled();
    });

    test('another language still reports a message its catalog lacks', () => {
        const error = rstest.spyOn(console, 'error').mockImplementation(() => {});

        intlFor({ tag: 'zh', messages: {} }).formatMessage({ id: 'save', defaultMessage: 'Save' });

        expect(error).toHaveBeenCalled();
    });
});

describe('bootLocale', () => {
    afterEach(() => {
        Reflect.deleteProperty(window, 'LANGUAGE');
    });

    test('a language without a tag boots in English', () => {
        Object.assign(window, { LANGUAGE: { messages: { save: 'Uložit' } } });

        expect(bootLocale()).toBeNull();
    });

    test('a catalog that is not an object is left out, the language kept', () => {
        Object.assign(window, { LANGUAGE: { tag: 'cs', messages: 'Uložit' } });

        expect(bootLocale()).toEqual({ tag: 'cs', messages: undefined });
    });
});
