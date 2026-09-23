// Copyright (C) 2026  Braiins Systems s.r.o.
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
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { stubLocation } from '@/mocks/location';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import Root from '@/pages/Root';
import { LayoutWorkspace } from './LayoutWorkspace';

// The real routes pull in every page; logout only needs their navigate.
rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

// `mocks.service` wants every method typed; at runtime it only registers what we pass.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

// The store is shared between tests, so each one signs in with the password state it is about.
// Logging out ends the session only where there is a password, as on the device.
async function signIn(hasPassword: boolean): Promise<ReturnType<typeof rstest.fn>> {
    let authenticated = true;
    const logout = rstest.fn(() => {
        if (hasPassword) authenticated = false;
        return {};
    });
    registerMocks(pb.services.AuthenticationService, { logout, isAuthenticated: () => ({ value: authenticated }) });
    registerMocks(pb.services.SystemService, { hasPassword: () => ({ value: hasPassword }) });
    await store.fetchSessionInfo();
    return logout;
}

// Under Root, as in the app: it is what redirects once the session ends.
function renderLayout() {
    return render(
        <IntlProvider locale="en">
            <MemoryRouter initialEntries={['/display']}>
                <Routes>
                    <Route element={<Root />}>
                        <Route path="*" element={<LayoutWorkspace children={null} />} />
                    </Route>
                </Routes>
            </MemoryRouter>
        </IntlProvider>,
    );
}

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.clearAllMocks();
});

describe('LayoutWorkspace logout', () => {
    test("next to boser, logout shows without a password and leaves for boser's login", async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: true }));
        // Without a password the session survives the logout.
        const logout = await signIn(false);
        const assign = rstest.fn();
        stubLocation({ assign });
        renderLayout();

        fireEvent.click(screen.getByRole('button', { name: 'Logout' }));

        await waitFor(() => expect(assign).toHaveBeenCalledWith('/bos/login'));
        expect(logout).toHaveBeenCalledOnce();
    });

    test('a standalone Deck without a password offers no logout', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: false }));
        await signIn(false);
        renderLayout();

        expect(screen.queryByRole('button', { name: 'Logout' })).toBeNull();
    });

    test('a standalone Deck with a password logs out to its own login', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: false }));
        const logout = await signIn(true);
        const assign = rstest.fn();
        stubLocation({ assign });
        const { default: router } = await import('@/routes');
        renderLayout();

        fireEvent.click(screen.getByRole('button', { name: 'Logout' }));

        await waitFor(() => expect(router.navigate).toHaveBeenCalledWith('/login'));
        expect(router.navigate).toHaveBeenCalledOnce();
        expect(logout).toHaveBeenCalledOnce();
        expect(assign).not.toHaveBeenCalled();
    });
});
