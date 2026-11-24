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

import { afterEach, expect, rstest, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { IntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import DisplayCombined from './DisplayCombined';
import type { CombinedSceneViewProps } from './components/CombinedSceneView/CombinedSceneView';
import type { ServiceMocks } from '@/lib/proto';
import * as pb from '@/proto';
import { store } from '@/store';
import { deckCapabilities } from './capabilities.fixture';
import { mocks } from '@/proto/transport';
import { paramDef } from './fn/test-helpers';

const TARGET = pb.create(pb.WidgetPositionSchema, { row: 1, col: 2 });

// jsdom lays nothing out, so a drag cannot reach a slot;
// the stub drops the first widget at TARGET.
function DropOnTarget({ widgets, onWidgetMove }: CombinedSceneViewProps) {
    return (
        <button
            type="button"
            onClick={() => onWidgetMove(widgets[0], { id: 'slot', position: TARGET, size: widgets[0].size })}
        >
            drop
        </button>
    );
}

rstest.mock('./components/CombinedSceneView/CombinedSceneView', () => ({ CombinedSceneView: DropOnTarget }));

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

afterEach(() => {
    cleanup();
    mocks.clear();
    store.setHardwareCapabilities(null);
});

test('a move leaves the stored params to the server', async () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'ticker',
        name: 'Ticker',
        supportedSizes: [pb.WidgetSize.SMALL],
        params: [paramDef('paramString', 'symbol'), paramDef('paramInteger', 'precision', true)],
    });
    // Saved before the package added `precision`.
    const widget = pb.create(pb.WidgetSchema, {
        id: 'widget-1',
        position: pb.create(pb.WidgetPositionSchema, { row: 0, col: 0 }),
        size: pb.WidgetSize.SMALL,
        config: pb.create(pb.WidgetConfigSchema, {
            widgetUid: manifest.uid,
            params: pb.create(pb.FieldValuesSchema, {
                fields: { symbol: { kind: { case: 'stringValue', value: 'BTC' } } },
            }),
        }),
    });
    const scene = pb.create(pb.SceneSchema, {
        id: 'scene-1',
        enabled: true,
        kind: { case: 'combined', value: pb.create(pb.Scene_CombinedSchema, { widgets: [widget] }) },
    });
    const updates: pb.UpdateWidgetRequest[] = [];
    registerMocks(pb.services.SceneManagementService, {
        getScene: () => ({ scene, runningWidgetCount: 1, maxRunningWidgetCount: 56 }),
        getAvailableWidgets: () => ({ widgets: [manifest] }),
        previewScene: () => (async function* () {})(),
        updateWidget: ({ req }) => {
            updates.push(req);
            return {};
        },
    });
    store.setHardwareCapabilities(deckCapabilities({ combinedScenesSupported: true }));
    registerMocks(pb.services.AccountManagementService, { getAllAccounts: () => ({ accounts: [] }) });
    registerMocks(pb.services.CredentialManagementService, { getCredentialTypes: () => ({ credentialTypes: [] }) });

    render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter initialEntries={['/display/scene-1']}>
                    <Routes>
                        <Route path="/display/:id" element={<DisplayCombined />} />
                    </Routes>
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );

    await screen.findByText('Running widgets: 1 / 56');
    fireEvent.click(screen.getByRole('button', { name: 'drop' }));

    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0].position).toEqual(TARGET);
    // Sent back, the stored params lack `precision`, which the server refuses.
    expect(updates[0].params).toBeUndefined();
});
