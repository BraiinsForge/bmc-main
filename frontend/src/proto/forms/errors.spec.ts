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
import { Code, ConnectError } from '@connectrpc/connect';
import { create } from '@bufbuild/protobuf';

import { proto } from '@/mocks/proto';
import { BadRequestSchema } from '@/proto/gen/google/rpc/error_details_pb';
import { collectAllErrors, collectAllErrorsAsFormattedList, errorMessage, parseFormErrors } from './errors';

const FALLBACK = 'Failed to save!';
const lost = proto.lostConnection();
const busy = new ConnectError('The device is busy.', Code.Unavailable);

// A refusal with no sentence of its own, only the one field's reason
function refusal(field: string, description: string): ConnectError {
    const detail = create(BadRequestSchema, { fieldViolations: [{ field, description }] });
    return new ConnectError('', Code.InvalidArgument, undefined, [{ desc: BadRequestSchema, value: detail }]);
}

afterEach(() => {
    rstest.restoreAllMocks();
});

describe('errorMessage', () => {
    test("a lost connection carries none of the browser's text and is not logged", () => {
        const log = rstest.spyOn(console, 'error').mockImplementation(() => {});

        expect(errorMessage(lost)).toBeUndefined();
        expect(log).not.toHaveBeenCalled();
    });

    test("the device's own status keeps its words and is not logged", () => {
        const log = rstest.spyOn(console, 'error').mockImplementation(() => {});

        expect(errorMessage(busy)).toBe('The device is busy.');
        expect(log).not.toHaveBeenCalled();
    });

    test("a bug in the page's own code keeps its text and is logged", () => {
        const log = rstest.spyOn(console, 'error').mockImplementation(() => {});
        const bug = new TypeError('row is undefined');

        expect(errorMessage(bug)).toBe('row is undefined');
        expect(log).toHaveBeenCalledWith(bug);
    });
});

describe('parseFormErrors', () => {
    test('a lost connection leaves only the fallback', () => {
        expect(parseFormErrors(lost, { knownFields: [], fallback: FALLBACK })).toEqual({
            global: [FALLBACK],
            fields: {},
        });
    });

    test("the device's own words keep the fallback out", () => {
        expect(parseFormErrors(busy, { knownFields: [], fallback: FALLBACK }).global).toEqual(['The device is busy.']);
    });

    test('a refusal of a drawn field alone keeps the fallback out', () => {
        expect(
            parseFormErrors(refusal('name', 'Missing value!'), { knownFields: ['name'], fallback: FALLBACK }),
        ).toEqual({
            global: [],
            fields: { name: ['Missing value!'] },
        });
    });

    test.each([
        ['a drawn', ['name']],
        ['an undrawn', []],
    ])('a blank reason for %s field gives the fallback', (_, knownFields) => {
        expect(parseFormErrors(refusal('name', '  '), { knownFields, fallback: FALLBACK })).toEqual({
            global: [FALLBACK],
            fields: {},
        });
    });
});

describe('collectAllErrors', () => {
    test('a lost connection gives the fallback, so the page words it', () => {
        expect(collectAllErrors(lost, FALLBACK)).toEqual([FALLBACK]);
    });

    test('an unavailable device keeps its words', () => {
        expect(collectAllErrors(busy, FALLBACK)).toEqual(['The device is busy.']);
    });

    test('as a formatted list, a lost connection gives the fallback', () => {
        expect(collectAllErrorsAsFormattedList(lost, FALLBACK)).toBe(FALLBACK);
    });
});
