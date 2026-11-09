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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, render, screen } from '@testing-library/react/pure';
import { useEffect } from 'react';
import { RawIntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import { URLS } from '@/constants';
import { intlFor } from '@/lib/intl';
import type { ServiceMocks } from '@/lib/proto';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import WorkspaceContainer from './Container';

rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.restoreAllMocks();
    store.setHardwareCapabilities(null);
});

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

/** The session check's answer, as the store holds it after asking the device. */
async function session(isAuthenticated: boolean): Promise<void> {
    registerMocks(pb.services.AuthenticationService, { isAuthenticated: () => ({ value: isAuthenticated }) });
    registerMocks(pb.services.SystemService, { hasPassword: () => ({ value: true }) });
    rstest.spyOn(store, 'loadFormats').mockResolvedValue();
    await store.fetchSessionInfo();
}

function Mounted({ onMount }: { onMount(): void }) {
    useEffect(onMount, [onMount]);
    return null;
}

function renderPage(onMount: () => void) {
    render(
        <RawIntlProvider value={intlFor({ tag: 'en' })}>
            <MemoryRouter initialEntries={[URLS.pages.display.list]}>
                <Routes>
                    <Route element={<WorkspaceContainer />}>
                        <Route path={URLS.pages.display.list} element={<Mounted onMount={onMount} />} />
                    </Route>
                </Routes>
            </MemoryRouter>
        </RawIntlProvider>,
    );
}

describe('WorkspaceContainer', () => {
    test('a page does not mount without a session', async () => {
        store.setHardwareCapabilities(deckCapabilities());
        await session(false);
        const mounted = rstest.fn();

        renderPage(mounted);

        expect(mounted).not.toHaveBeenCalled();
        expect(screen.getByRole('navigation', { name: 'Side navigation' }), 'the layout still shows').toBeTruthy();
    });

    test('a page mounts with a session', async () => {
        store.setHardwareCapabilities(deckCapabilities());
        await session(true);
        const mounted = rstest.fn();

        renderPage(mounted);

        expect(mounted).toHaveBeenCalledOnce();
    });
});
