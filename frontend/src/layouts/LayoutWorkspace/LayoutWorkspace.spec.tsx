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
import { MemoryRouter } from 'react-router';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import { LayoutWorkspace } from './LayoutWorkspace';

// jsdom has no matchMedia, which Carbon's SideNav queries for its breakpoint.
window.matchMedia ??= (query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList;

// `mocks.service` wants every method typed; at runtime it only registers what we pass.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

function renderLayout() {
    return render(
        <IntlProvider locale="en">
            <MemoryRouter>
                <LayoutWorkspace children={null} />
            </MemoryRouter>
        </IntlProvider>,
    );
}

// jsdom won't let us spy on location.assign directly; swap the whole location.
const REAL_LOCATION = window.location;
function stubAssign(): ReturnType<typeof rstest.fn> {
    const assign = rstest.fn();
    Object.defineProperty(window, 'location', {
        configurable: true,
        value: Object.assign(new URL(window.location.href), { assign }),
    });
    return assign;
}

afterEach(() => {
    cleanup();
    mocks.clear();
    Object.defineProperty(window, 'location', { configurable: true, value: REAL_LOCATION });
});

describe('LayoutWorkspace logout', () => {
    test("next to boser, logout shows without a password and leaves for boser's login", async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: true }));
        const logout = rstest.fn(() => ({}));
        registerMocks(pb.services.AuthenticationService, { logout });
        const assign = stubAssign();
        renderLayout();

        fireEvent.click(screen.getByRole('button', { name: 'Logout' }));

        await waitFor(() => expect(assign).toHaveBeenCalledWith('/bos/login'));
        expect(logout).toHaveBeenCalledOnce();
    });

    test('a standalone Deck without a password offers no logout', () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: false }));
        renderLayout();

        expect(screen.queryByRole('button', { name: 'Logout' })).toBeNull();
    });
});
