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

import { beforeEach, describe, expect, test } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import * as pb from '@/proto';
import { CredentialTypeForm } from './CredentialTypeForm';

beforeEach(cleanup);

const credentialType = (allowHosts: string[], description = 'A token.') =>
    pb.create(pb.CredentialTypeSchema, {
        id: 'demo',
        name: 'Demo',
        description,
        fields: [],
        egress: allowHosts.length
            ? pb.create(pb.EgressPolicySchema, {
                  pin: { case: 'hosts', value: pb.create(pb.HostPinSchema, { allowHosts }) },
              })
            : undefined,
    });

const addressPinnedType = () =>
    pb.create(pb.CredentialTypeSchema, {
        id: 'demo',
        name: 'Demo',
        description: 'A service.',
        fields: [pb.create(pb.ManifestParamDefinitionSchema, { key: 'url', name: 'Address' })],
        egress: pb.create(pb.EgressPolicySchema, {
            pin: { case: 'address', value: pb.create(pb.AddressPinSchema, { field: 'url' }) },
        }),
    });

const renderForm = (type: pb.CredentialType, values: Record<string, string> = {}) =>
    render(
        <IntlProvider locale="en">
            <CredentialTypeForm type={type} values={values} onChange={() => {}} />
        </IntlProvider>,
    );

describe('CredentialTypeForm egress disclosure', () => {
    test('names the hosts a pinned type is limited to', () => {
        const { queryByText } = renderForm(credentialType(['api.braiins.com']));

        expect(queryByText('It is only ever sent to api.braiins.com.')).toBeTruthy();
    });

    test('says so when a type is unpinned', () => {
        const { queryByText } = renderForm(credentialType([]));

        expect(queryByText('It may be sent to any host.')).toBeTruthy();
    });

    test('a description claiming a pin does not override an absent policy', () => {
        const { queryByText } = renderForm(credentialType([], 'Only ever sent to totally-safe.example.'));

        expect(queryByText('It may be sent to any host.')).toBeTruthy();
    });
});

describe('CredentialTypeForm address pin', () => {
    test('names the address field a type is pinned to, never any host', () => {
        const { queryByText } = renderForm(addressPinnedType());

        expect(queryByText('It is only ever sent to the address you enter in Address.')).toBeTruthy();
        expect(queryByText('It may be sent to any host.')).toBeNull();
    });
});
