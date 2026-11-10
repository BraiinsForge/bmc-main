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
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react/pure';
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
    const option = await screen.findByRole('option', { name });
    await act(async () => {
        fireEvent.click(option);
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

const dropdown = () => screen.getByRole('combobox', { name: 'Network' });

async function pickListedHomeNet(): Promise<void> {
    await waitFor(() => expect(dropdown().textContent).toContain('OfficeNet'));
    await clickButton('Refresh');
    await waitFor(() => expect(screen.queryByText('Scanning for networks…')).toBeNull());
    await chooseNetwork('HomeNet');
    expect(dropdown().textContent).toContain('OfficeNet');
}

async function renderPage(methods: Partial<ServiceMocks<typeof pb.NetworkService>>): Promise<void> {
    store.setHardwareCapabilities(deckCapabilities());
    const defaults: Partial<ServiceMocks<typeof pb.NetworkService>> = {
        getNetworkInfo: () => pb.create(pb.NetworkInfoResponseSchema),
        getNetworkConfig: () => pb.create(pb.NetworkConfigSchema),
        getWifiStatus: () =>
            pb.create(pb.WifiStatusResponseSchema, {
                status: { network: { ssid: 'OfficeNet', encryptionType: pb.EncryptionType.WPA2 } },
            }),
        getWifiSavedNetworks: () => pb.create(pb.WifiSavedNetworksResponseSchema),
        scanWifi: () =>
            pb.create(pb.ScanWifiResponseSchema, {
                networks: [
                    pb.create(pb.WifiNetworkSchema, { ssid: 'HomeNet', encryptionType: pb.EncryptionType.WPA2 }),
                ],
            }),
    };
    mocks.service(pb.NetworkService, { ...defaults, ...methods } as ServiceMocks<typeof pb.NetworkService>);

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
}

function teardown(): void {
    cleanup();
    mocks.clear();
    store.setHardwareCapabilities(null);
    rstest.restoreAllMocks();
}

describe('<Network /> a join the device accepts', () => {
    afterEach(teardown);

    test('shows the joined network, and no error, when the status reload after it fails', async () => {
        let statusAsked = 0;
        await renderPage({
            getWifiStatus: () => {
                statusAsked += 1;
                if (statusAsked > 1) throw new ConnectError('the device moved', Code.Unavailable);
                return pb.create(pb.WifiStatusResponseSchema, {
                    status: { network: { ssid: 'OfficeNet', encryptionType: pb.EncryptionType.WPA2 } },
                });
            },
            setWifi: () => pb.create(pb.EmptySchema),
        });
        await waitFor(() => expect(dropdown().textContent).toContain('OfficeNet'));
        await chooseNetwork('Other…');
        await changeInput('wifi-manual-ssid', 'TestNet');

        await clickButton('Join');

        await waitFor(() => expect(statusAsked).toBe(2));
        // Lets the failed reload settle, so an error it raised would show
        await act(async () => {});
        expect(dropdown().textContent).toContain('TestNet');
        expect(screen.queryByText(/the device moved/)).toBeNull();
    });
});

describe('<Network /> a join the device never answers', () => {
    afterEach(teardown);

    test('unlocks the page and tells where the device may have gone', async () => {
        const showToast = rstest.spyOn(toast, 'show');
        const showError = rstest.spyOn(toast, 'error');
        await renderPage({
            setWifi: () => {
                throw new ConnectError('deadline exceeded', Code.DeadlineExceeded);
            },
        });
        await waitFor(() => expect(dropdown().textContent).toContain('OfficeNet'));
        await chooseNetwork('Other…');
        await changeInput('wifi-manual-ssid', 'TestNet');
        await changeInput('wifi-manual-security', pb.EncryptionType.WPA2);
        await changeInput('wifi-manual-password', 'hunter22');

        await clickButton('Join');

        await waitFor(() =>
            expect(showToast).toHaveBeenCalledWith(
                'warning',
                'The Deck may have switched to TestNet. If this page stops responding, connect to TestNet and open the Deck there.',
                { duration: Number.POSITIVE_INFINITY },
            ),
        );
        expect(showError).not.toHaveBeenCalled();
        // Closing clears the password, where a refusal keeps it for another try
        await waitFor(() => expect(document.getElementById('wifi-manual-password')).toHaveProperty('value', ''));
        expect(document.getElementById('wifi-manual-ssid')).toHaveProperty('disabled', false);
    });
});

describe('<Network /> a join the device refuses', () => {
    beforeEach(() =>
        renderPage({
            setWifi: () => {
                throw new ConnectError(REFUSAL, Code.InvalidArgument);
            },
        }),
    );

    afterEach(teardown);

    test('shows the sentence from the manual dialog, which unlocks', async () => {
        const showError = rstest.spyOn(toast, 'error');
        await waitFor(() => expect(dropdown().textContent).toContain('OfficeNet'));
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
        await pickListedHomeNet();
        await changeInput('wifi-password-entry', 'wrong-password');

        await clickButton('Connect');

        await waitFor(() => expect(showError).toHaveBeenCalledWith(REFUSAL));
        expect(screen.getByRole('button', { name: 'Connect' })).toBeTruthy();
    });

    test('keeps the dropdown on the current network when a listed one is refused', async () => {
        const showError = rstest.spyOn(toast, 'error');
        await pickListedHomeNet();
        await changeInput('wifi-password-entry', 'wrong-password');

        await clickButton('Connect');

        await waitFor(() => expect(showError).toHaveBeenCalledWith(REFUSAL));
        expect(dropdown().textContent).toContain('OfficeNet');
    });

    test('keeps the dropdown on the current network when a listed one is cancelled', async () => {
        await pickListedHomeNet();
        const dialog = screen.getByRole('heading', { name: 'Password for HomeNet' }).closest('[role="dialog"]');
        if (!(dialog instanceof HTMLElement)) throw new Error('BUG: the password heading sits outside a dialog');

        await act(async () => {
            fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
        });

        expect(dropdown().textContent).toContain('OfficeNet');
    });
});
