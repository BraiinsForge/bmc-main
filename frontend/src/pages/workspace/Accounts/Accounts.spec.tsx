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

import { afterEach, describe, expect, it } from '@rstest/core';
import { cleanup, render, screen } from '@testing-library/react/pure';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { IntlProvider } from 'react-intl';
import { MemoryRouter } from 'react-router';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import PageAccounts from './Accounts';

// `mocks.service` wants every method typed; at runtime it only registers what we pass.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

afterEach(cleanup);

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter>
                    <PageAccounts />
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );
}

describe('Accounts page', () => {
    it('lists accounts from the backend and resolves the credential-type name', async () => {
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({
                accounts: [pb.create(pb.AccountSchema, { id: '1', typeId: 'braiins-pool', name: 'My Pool' })],
            }),
        });
        registerMocks(pb.services.CredentialManagementService, {
            getCredentialTypes: () => ({
                credentialTypes: [pb.create(pb.CredentialTypeSchema, { id: 'braiins-pool', name: 'Braiins Pool' })],
            }),
        });

        renderPage();

        // jsdom reports width 0, so the table renders its compact layout; the type column (the
        // resolved credential-type name) is always shown, proving the fetched row rendered.
        expect(await screen.findByText('Braiins Pool')).toBeTruthy();
    });

    it('shows the empty state when there are no accounts', async () => {
        registerMocks(pb.services.AccountManagementService, { getAllAccounts: () => ({ accounts: [] }) });
        registerMocks(pb.services.CredentialManagementService, { getCredentialTypes: () => ({ credentialTypes: [] }) });

        renderPage();

        expect(await screen.findByText('No Connected Accounts Yet.')).toBeTruthy();
    });
});
