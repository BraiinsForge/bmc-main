// Copyright (C) 2025  Braiins Systems s.r.o.
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

import type { BadRequest_FieldViolation } from '../gen/google/rpc/error_details_pb';
import { parseFieldPath, parseFieldViolations } from './parseFieldViolation';

describe('parseFieldPath', () => {
    test('field name only', () => {
        expect(parseFieldPath('field')).toEqual(['field']);
    });

    test('array index', () => {
        expect(parseFieldPath('field[0]')).toEqual(['field', '0']);
    });

    test('multiple array indices', () => {
        expect(parseFieldPath('field[0][1]')).toEqual(['field', '0', '1']);
    });

    test('nested path', () => {
        expect(parseFieldPath('field.nested')).toEqual(['field', 'nested']);
    });

    test('complex nested path', () => {
        expect(parseFieldPath('emailAddresses[3].type[2]')).toEqual(['emailAddresses', '3', 'type', '2']);
    });

    test('empty array for empty string', () => {
        expect(parseFieldPath('')).toEqual([]);
    });

    test('a snake_case field name becomes camelCase', () => {
        expect(parseFieldPath('dns_servers[0]')).toEqual(['dnsServers', '0']);
    });

    test('a map key keeps its wire spelling', () => {
        expect(parseFieldPath('params["show_date"]')).toEqual(['params', 'show_date']);
    });

    test('an unquoted map key keeps its wire spelling too', () => {
        expect(parseFieldPath('params[show_date]')).toEqual(['params', 'show_date']);
    });

    test('a lone quote inside a key is kept', () => {
        expect(parseFieldPath(`params["it's"]`)).toEqual(['params', "it's"]);
    });

    test('a dot inside a key does not split it', () => {
        expect(parseFieldPath('params["a.b"]')).toEqual(['params', 'a.b']);
    });

    test('an opening bracket inside a key is kept', () => {
        expect(parseFieldPath('params["a[b"]')).toEqual(['params', 'a[b']);
    });

    test('empty brackets are dropped', () => {
        expect(parseFieldPath('params[]')).toEqual(['params']);
    });

    test('repeated separators collapse', () => {
        expect(parseFieldPath('a..b')).toEqual(['a', 'b']);
    });
});

describe('parseFieldViolations', () => {
    test('parses all fields', () => {
        const input: BadRequest_FieldViolation[] = [
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'address',
                description: "address '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'netmask',
                description: "netmask 'abdf' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'gateway',
                description: "gateway '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'dns_servers[0]',
                description: "dns_servers[0] '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'dns_servers[1]',
                description: "dns_servers[1] '4567' does not have valid IPv4 format",
                reason: '',
            },
        ];
        const output = {
            address: ["'1234' does not have valid IPv4 format"],
            dnsServers: [["'1234' does not have valid IPv4 format"], ["'4567' does not have valid IPv4 format"]],
            gateway: ["'1234' does not have valid IPv4 format"],
            netmask: ["'abdf' does not have valid IPv4 format"],
        };

        expect(parseFieldViolations(input).parsed).toEqual(output);
    });
    test('respects known fields', () => {
        const input: BadRequest_FieldViolation[] = [
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'address',
                description: "address '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'netmask',
                description: "netmask 'abdf' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'gateway',
                description: "gateway '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'dns_servers[0]',
                description: "dns_servers[0] '1234' does not have valid IPv4 format",
                reason: '',
            },
            {
                $typeName: 'google.rpc.BadRequest.FieldViolation',
                field: 'dns_servers[1]',
                description: "dns_servers[1] '4567' does not have valid IPv4 format",
                reason: '',
            },
        ];
        const outputParsed = {
            address: ["'1234' does not have valid IPv4 format"],
            gateway: ["'1234' does not have valid IPv4 format"],
        };
        const outputUnmatched = [
            "netmask 'abdf' does not have valid IPv4 format",
            "dns_servers[0] '1234' does not have valid IPv4 format",
            "dns_servers[1] '4567' does not have valid IPv4 format",
        ];

        const { parsed, unmatched } = parseFieldViolations(input, ['address', 'gateway']);

        expect(parsed).toEqual(outputParsed);
        expect(unmatched).toEqual(outputUnmatched);
    });
});

describe('parseFieldViolations entry keys', () => {
    const violation = (field: string, description: string): BadRequest_FieldViolation => ({
        $typeName: 'google.rpc.BadRequest.FieldViolation',
        field,
        description,
        reason: '',
    });

    test('a map key keeps its wire spelling', () => {
        const input = [violation('params["show_date"]', 'Must be true or false')];

        expect(parseFieldViolations(input, ['params']).parsed).toEqual({
            params: { show_date: ['Must be true or false'] },
        });
    });

    test('a single-word map key is unaffected either way', () => {
        const input = [violation('credential_bindings["pool"]', 'Account not found')];

        expect(parseFieldViolations(input, ['credentialBindings']).parsed).toEqual({
            credentialBindings: { pool: ['Account not found'] },
        });
    });

    test('a field name outside brackets is still camelCased', () => {
        const input = [violation('dns_servers[0]', 'bad address')];

        expect(parseFieldViolations(input, ['dnsServers']).parsed).toEqual({
            dnsServers: [['bad address']],
        });
    });

    test('an unquoted map key keeps its wire spelling too', () => {
        const input = [violation('params[show_date]', 'Must be true or false')];

        expect(parseFieldViolations(input, ['params']).parsed).toEqual({
            params: { show_date: ['Must be true or false'] },
        });
    });
});
