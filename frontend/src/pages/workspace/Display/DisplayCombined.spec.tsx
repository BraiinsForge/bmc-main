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

import { afterEach, beforeEach, describe, expect, rstest, test } from '@rstest/core';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { Code, ConnectError } from '@connectrpc/connect';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { IntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import DisplayCombined from './DisplayCombined';
import type { ServiceMocks } from '@/lib/proto';
import { Toaster } from '@/lib/toast';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

const LIMIT_ERROR = 'running widget limit exceeded: 56 running, operation would activate 1, maximum 56';
const LIMIT_MESSAGE = 'Running widget limit reached.';
const manifest = pb.create(pb.WidgetManifestSchema, {
    uid: 'clock',
    name: 'Clock',
    supportedSizes: [pb.WidgetSize.SMALL],
});
const scene = pb.create(pb.SceneSchema, {
    id: 'scene-1',
    enabled: true,
    kind: {
        case: 'combined',
        value: pb.create(pb.Scene_CombinedSchema, { widgets: [] }),
    },
});

function installMocks(): void {
    mocks.clear();
    registerMocks(pb.services.SceneManagementService, {
        getScene: () => ({ scene, runningWidgetCount: 56, maxRunningWidgetCount: 56 }),
        getAvailableWidgets: () => ({ widgets: [manifest] }),
        previewScene: () => (async function* () {})(),
        addWidget: () => {
            throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
        },
    });
    registerMocks(pb.services.HardwareService, {
        getHardwareCapabilities: () => ({ combinedScenesSupported: true }),
    });
    registerMocks(pb.services.AccountManagementService, { getAllAccounts: () => ({ accounts: [] }) });
    registerMocks(pb.services.CredentialManagementService, { getCredentialTypes: () => ({ credentialTypes: [] }) });
    registerMocks(pb.services.SystemService, { getTimezoneList: () => ({ timezones: [] }) });
}

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter initialEntries={['/display/scene-1']}>
                    <Routes>
                        <Route path="/display/:id" element={<DisplayCombined />} />
                    </Routes>
                    <Toaster />
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );
}

beforeEach(installMocks);

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.useRealTimers();
});

describe('running widget limit', () => {
    test('an accepted preview refreshes the running widget counter', async () => {
        rstest.useFakeTimers();
        let getSceneCalls = 0;
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({
                scene,
                runningWidgetCount: getSceneCalls++ === 0 ? 50 : 56,
                maxRunningWidgetCount: 56,
            }),
            previewScene: () =>
                (async function* () {
                    yield pb.create(pb.EmptySchema);
                })(),
        });

        renderPage();
        await act(async () => {
            await rstest.runAllTimersAsync();
        });
        // The preview continuation settles after the first drain, then schedules the debounced reload.
        await act(async () => {
            await rstest.runAllTimersAsync();
        });

        expect(screen.getByText('Running widgets: 56 / 56')).toBeTruthy();
    });

    test('a rejected preview explains the running widget limit', async () => {
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({
                scene: { ...scene, enabled: false },
                runningWidgetCount: 56,
                maxRunningWidgetCount: 56,
            }),
            previewScene: () =>
                (async function* () {
                    yield* [];
                    throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
                })(),
        });

        renderPage();

        await waitFor(() => expect(document.body.textContent).toContain(LIMIT_MESSAGE));
        expect(screen.queryByText('Edit Combined Scene')).toBeNull();
    });

    test('a preview connection failure after admission keeps the editor open', async () => {
        registerMocks(pb.services.SceneManagementService, {
            previewScene: () =>
                (async function* () {
                    yield pb.create(pb.EmptySchema);
                    throw new ConnectError('connection lost', Code.Unavailable);
                })(),
        });

        renderPage();

        await waitFor(() => expect(document.body.textContent).toContain('Display preview connection lost!'));
        expect(screen.getByText('Edit Combined Scene')).toBeTruthy();
    });

    test('a rejected combined add does not open the widget editor and explains the running widget limit', async () => {
        const { container } = renderPage();

        await screen.findByText('Running widgets: 56 / 56');
        const addButton = container.querySelector<HTMLButtonElement>('main button:not([title])');
        if (!addButton) throw new Error('combined-scene add button not rendered');
        fireEvent.click(addButton);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));

        await waitFor(() => expect(document.body.textContent).toContain(LIMIT_MESSAGE));
        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
    });
});
