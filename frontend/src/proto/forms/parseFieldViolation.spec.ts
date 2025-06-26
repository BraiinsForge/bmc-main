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
