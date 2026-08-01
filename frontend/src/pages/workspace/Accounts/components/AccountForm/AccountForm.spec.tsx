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
import { AccountForm } from './AccountForm';

beforeEach(cleanup);

const DESTINATIONS = 'Allowed destinations (optional)';

const credentialTypes = (allowHosts: string[]) =>
    new Map([
        [
            'demo',
            pb.create(pb.CredentialTypeSchema, {
                id: 'demo',
                name: 'Demo',
                description: 'A token.',
                fields: [],
                egress: allowHosts.length ? pb.create(pb.EgressPolicySchema, { allowHosts }) : undefined,
            }),
        ],
    ]);

const renderForm = (typePin: string[], allowHostsError?: string) =>
    render(
        <IntlProvider locale="en">
            <AccountForm
                mode="create"
                credentialTypes={credentialTypes(typePin)}
                type={{ value: 'demo', onChange: () => {} }}
                name={{ value: '', onChange: () => {} }}
                fieldValues={{}}
                onFieldChange={() => {}}
                allowHosts={{ value: '', onChange: () => {}, error: allowHostsError }}
            />
        </IntlProvider>,
    );

describe('AccountForm destination control', () => {
    test('lets the operator name destinations for a type that pins none', () => {
        expect(renderForm([]).queryByLabelText(DESTINATIONS)).toBeTruthy();
    });

    test('offers no control for a type that pins its own', () => {
        // Not merely disabled: a pinned type's destinations are fixed,
        // so the form has nothing for the operator to set.
        expect(renderForm(['api.braiins.com']).queryByLabelText(DESTINATIONS)).toBeNull();
    });

    test('shows a rejected entry against the field it came from', () => {
        // The server reports `Line N`, and that only means something
        // beside the textarea those lines were typed into.
        const { queryByText } = renderForm([], 'Line 2: an entry cannot contain spaces');

        expect(queryByText('Line 2: an entry cannot contain spaces')).toBeTruthy();
    });
});
