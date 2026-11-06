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
import { MemoryRouter } from 'react-router';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { toast } from '@/lib/toast';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';

import NetworkPage from './Network';

const REFUSAL = 'The password for HomeNet was rejected.';

async function chooseNetwork(name: string): Promise<void> {
    await act(async () => {
        fireEvent.click(screen.getByRole('combobox', { name: 'Network' }));
    });
    await act(async () => {
        fireEvent.click(screen.getByRole('option', { name }));
    });
}

async function changeInput(id: string, value: string | number): Promise<void> {
    const input = document.getElementById(id);
    if (input == null) throw new Error(`BUG: the page has no input ${id}`);
    await act(async () => {
        fireEvent.change(input, { target: { value } });
    });
}

async function clickButton(name: string): Promise<void> {
    await act(async () => {
        fireEvent.click(screen.getByRole('button', { name }));
    });
}

describe('<Network /> a join the device refuses', () => {
    beforeEach(async () => {
        store.setHardwareCapabilities(deckCapabilities());
        const methods: Partial<ServiceMocks<typeof pb.NetworkService>> = {
            getNetworkInfo: () => pb.create(pb.NetworkInfoResponseSchema),
            getNetworkConfig: () => pb.create(pb.NetworkConfigSchema),
            getWifiStatus: () => pb.create(pb.WifiStatusResponseSchema),
            getWifiSavedNetworks: () => pb.create(pb.WifiSavedNetworksResponseSchema),
            scanWifi: () =>
                pb.create(pb.ScanWifiResponseSchema, {
                    networks: [
                        pb.create(pb.WifiNetworkSchema, { ssid: 'HomeNet', encryptionType: pb.EncryptionType.WPA2 }),
                    ],
                }),
            setWifi: () => {
                throw new ConnectError(REFUSAL, Code.InvalidArgument);
            },
        };
        mocks.service(pb.NetworkService, methods as ServiceMocks<typeof pb.NetworkService>);

        render(
            <HelmetProvider>
                <IntlProvider locale="en">
                    <MemoryRouter>
                        <NetworkPage />
                    </MemoryRouter>
                </IntlProvider>
            </HelmetProvider>,
        );
        await waitFor(() => expect(screen.getByRole('combobox', { name: 'Network' })).toBeTruthy());
    });

    afterEach(() => {
        cleanup();
        mocks.clear();
        store.setHardwareCapabilities(null);
        rstest.restoreAllMocks();
    });

    test('shows the sentence from the manual dialog, which unlocks', async () => {
        const showError = rstest.spyOn(toast, 'error');
        await chooseNetwork('Other…');
        await changeInput('wifi-manual-ssid', 'HomeNet');
        await changeInput('wifi-manual-security', pb.EncryptionType.WPA2);
        await changeInput('wifi-manual-password', 'wrong-password');

        await clickButton('Join');

        await waitFor(() => expect(showError).toHaveBeenCalledWith(REFUSAL));
        expect(screen.getByRole('button', { name: 'Join' })).toBeTruthy();
    });

    test('shows the sentence from the password dialog, which unlocks', async () => {
        const showError = rstest.spyOn(toast, 'error');
        await clickButton('Refresh');
        await waitFor(() => expect(screen.queryByText('Scanning for networks…')).toBeNull());
        await chooseNetwork('HomeNet');
        await changeInput('wifi-password-entry', 'wrong-password');

        await clickButton('Connect');

        await waitFor(() => expect(showError).toHaveBeenCalledWith(REFUSAL));
        expect(screen.getByRole('button', { name: 'Connect' })).toBeTruthy();
    });
});
