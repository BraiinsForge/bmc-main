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
import { Code, ConnectError } from '@connectrpc/connect';
import { create } from '@bufbuild/protobuf';

import { BadRequestSchema } from '@/proto/gen/google/rpc/error_details_pb';
import { mapManifestUpdateError } from './manifestFormErrors';

function badRequest(violations: Array<[field: string, description: string]>): ConnectError {
    const detail = create(BadRequestSchema, {
        fieldViolations: violations.map(([field, description]) => ({ field, description })),
    });
    return new ConnectError('Bad request', Code.InvalidArgument, undefined, [
        { desc: BadRequestSchema, value: detail },
    ]);
}

describe('mapManifestUpdateError', () => {
    test('a param violation lands on the param', () => {
        const errors = mapManifestUpdateError(badRequest([['params["color"]', 'Must be text']]));
        expect(errors.fields).toEqual({ color: ['Must be text'] });
    });

    test("an item violation lands on its index, apart from the list's own", () => {
        const errors = mapManifestUpdateError(
            badRequest([
                ['params["counts"]', 'Must have at most 2 items'],
                ['params["counts"][0]', 'Must be at most 5'],
                ['params["counts"][2]', 'Must be a whole number'],
            ]),
        );
        expect(errors.fields.counts).toEqual(['Must have at most 2 items']);
        expect(errors.items?.counts?.[0]).toEqual({ errors: ['Must be at most 5'] });
        expect(errors.items?.counts?.[1]).toBeUndefined();
        expect(errors.items?.counts?.[2]).toEqual({ errors: ['Must be a whole number'] });
    });

    test("an object row's field violation lands on that field of its row", () => {
        const errors = mapManifestUpdateError(
            badRequest([
                ['params["links"][1]["label"]', 'Value is required'],
                ['params["links"][1]', 'Must be an object'],
            ]),
        );
        expect(errors.items?.links?.[1]).toEqual({
            errors: ['Must be an object'],
            fields: { label: ['Value is required'] },
        });
    });

    test('a key named like an Object member keeps its violations', () => {
        const { fields, items, credentials } = mapManifestUpdateError(
            badRequest([
                ['params["constructor"]', 'Must be text'],
                ['params["toString"][0]["valueOf"]', 'Value is required'],
                ['credential_bindings["hasOwnProperty"]', 'Unknown account'],
            ]),
        );
        expect({ fields, items, credentials }).toEqual({
            fields: { constructor: ['Must be text'] },
            items: { toString: [{ fields: { valueOf: ['Value is required'] } }] },
            credentials: { hasOwnProperty: ['Unknown account'] },
        });
    });

    test('a binding violation lands on its slot', () => {
        const errors = mapManifestUpdateError(badRequest([['credential_bindings["pool"]', 'Unknown account']]));
        expect(errors.credentials).toEqual({ pool: ['Unknown account'] });
    });

    test('any other violation is global', () => {
        const errors = mapManifestUpdateError(
            badRequest([
                ['size', 'Unsupported size'],
                ['params["counts"]["x"]', 'Not an item'],
            ]),
        );
        expect(errors.global).toEqual(expect.arrayContaining(['Unsupported size', 'Not an item']));
        expect(errors.items).toEqual({});
    });
});
