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

import { describe, test, expect } from '@rstest/core';
import { Code, ConnectError } from '@connectrpc/connect';
import { create } from '@bufbuild/protobuf';

import { BadRequestSchema } from '@/proto/gen/google/rpc/error_details_pb';
import { toFormErrors } from './errors';

function badRequest(violations: Array<[field: string, description: string]>): ConnectError {
    const detail = create(BadRequestSchema, {
        fieldViolations: violations.map(([field, description]) => ({ field, description })),
    });
    return new ConnectError('Bad request', Code.InvalidArgument, undefined, [
        { desc: BadRequestSchema, value: detail },
    ]);
}

function expectErrors<T>(errors: null | T): T {
    expect(errors).not.toBeNull();
    return errors as T;
}

describe('toFormErrors', () => {
    test('a top-level violation lands on the form field it names', () => {
        const { fields } = expectErrors(toFormErrors(badRequest([['hostname', 'Too long!']])));
        expect(fields).toEqual({ hostname: ['Too long!'] });
    });

    test('a snake_case wire name maps onto the form field', () => {
        const { fields } = expectErrors(toFormErrors(badRequest([['timezone_id', 'invalid timezone variant']])));
        expect(fields).toEqual({ timezone: ['invalid timezone variant'] });
    });

    test('nested pool and network violations are flattened onto their fields', () => {
        const { fields } = expectErrors(
            toFormErrors(
                badRequest([
                    ['pool.url', 'Invalid URL!'],
                    ['network.protocol', 'Protocol must be specified!'],
                    ['network.static.address', 'Missing value!'],
                ]),
            ),
        );
        expect(fields).toEqual({
            poolUrl: ['Invalid URL!'],
            protocol: ['Protocol must be specified!'],
            staticAddress: ['Missing value!'],
        });
    });

    test('an indexed DNS server violation lands on the DNS field', () => {
        const { fields } = expectErrors(
            toFormErrors(badRequest([['network.static.dns_servers[1]', "'x' is not a valid IPv4!"]])),
        );
        expect(fields).toEqual({ staticDns: ["'x' is not a valid IPv4!"] });
    });

    test('a violation naming no form field goes global with the status message', () => {
        const { global, fields } = expectErrors(toFormErrors(badRequest([['pool.password', 'Missing value!']])));
        expect(fields).toEqual({});
        expect(global).toEqual(['Bad request', 'Missing value!']);
    });
});
