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
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
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

    const oneGenericType = () => {
        registerMocks(pb.services.CredentialManagementService, {
            getCredentialTypes: () => ({
                credentialTypes: [pb.create(pb.CredentialTypeSchema, { id: 'generic-token', name: 'Generic token' })],
            }),
        });
    };

    const allowHostsTextarea = async (): Promise<HTMLTextAreaElement> => {
        renderPage();
        // The header and the empty state both offer the button; either opens the dialog.
        fireEvent.click((await screen.findAllByText('Add New Account'))[0]);
        // Pick the type explicitly rather than racing the types fetch,
        // whose result seeds the default only if it arrives before the click.
        fireEvent.click(await screen.findByText('Generic token'));
        return (await screen.findByLabelText('Allowed destinations (optional)')) as HTMLTextAreaElement;
    };

    // In create mode the modal submit repeats the "Add New Account" label
    // of the page buttons, so it is found by its place in the modal footer
    // rather than by text; the close cross is the other primary button.
    const submitDialog = () => {
        const submit = document.body.querySelector('.cds--modal .cds--btn--primary:not(.cds--btn--icon-only)');
        if (!submit) throw new Error('BUG: the dialog submit must render');
        fireEvent.click(submit);
    };

    /// jsdom is laxer than a browser: an invalid `selectorPrimaryFocus` yields null
    /// here and throws there. Neither moves the focus, so assert where it lands.
    it('opens the add dialog with the focus inside the form', async () => {
        oneGenericType();
        registerMocks(pb.services.AccountManagementService, { getAllAccounts: () => ({ accounts: [] }) });

        renderPage();
        fireEvent.click((await screen.findAllByText('Add New Account'))[0]);
        await screen.findByLabelText('Account Name');

        expect(document.activeElement?.tagName).toBe('INPUT');
    });

    /// The server reports allow-hosts violations as "Line N" against the list it
    /// received, and that number is the operator's only pointer. Normalizing the
    /// textarea before the send — and freezing it while the save is in flight —
    /// makes the visible text the very list the server numbered.
    it('normalizes the allow-hosts textarea and freezes it while the save is in flight', async () => {
        oneGenericType();
        let sent: string[] | undefined;
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({ accounts: [] }),
            upsertAccount: ({ req, conf }) => {
                conf({ delay: 80 });
                sent = req.allowHosts;
                return { value: 'a-1' };
            },
        });

        const textarea = await allowHostsTextarea();
        fireEvent.change(textarea, { target: { value: ' a.example.com\n\nb.example.com\n' } });
        submitDialog();

        await waitFor(() => expect(textarea.disabled).toBe(true));
        expect(textarea.value).toBe('a.example.com\nb.example.com');
        expect(sent).toEqual(['a.example.com', 'b.example.com']);
    });

    it('unfreezes the normalized textarea when the save fails', async () => {
        oneGenericType();
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({ accounts: [] }),
            upsertAccount: () => {
                throw new Error('backend says no');
            },
        });

        const textarea = await allowHostsTextarea();
        fireEvent.change(textarea, { target: { value: ' a.example.com\n\nb.example.com\n' } });
        submitDialog();

        await waitFor(() => expect(textarea.disabled).toBe(false));
        expect(textarea.value).toBe('a.example.com\nb.example.com');
    });
});
