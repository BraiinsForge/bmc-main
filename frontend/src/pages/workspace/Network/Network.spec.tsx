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

import { afterEach, describe, test, expect } from '@rstest/core';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { MemoryRouter } from 'react-router';
import { RawIntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { ConnectError, Code } from '@connectrpc/connect';
import { create } from '@bufbuild/protobuf';

import Network from './Network';
import * as pb from '@/proto';
import { BadRequestSchema } from '@/proto/gen/google/rpc/error_details_pb';
import { mocks } from '@/proto/transport';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import { Toaster } from '@/lib/toast';
import type { ServiceMocks } from '@/lib/proto';
import { intlFor } from '@/lib/intl';

// The name the mocked device answers to; a successful rename replaces it, as the
// device's own configuration would.
let deviceHostname = 'braiins-deck';
let renameRequests: string[] = [];

type SetHostnameMock = ServiceMocks<typeof pb.services.NetworkService>['setHostname'];
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

const acceptRename: SetHostnameMock = ({ req }) => {
    renameRequests.push(req.hostname);
    deviceHostname = req.hostname;
    return {};
};

function renderPage(setHostname: SetHostnameMock = acceptRename) {
    mocks.clear();
    // Without Wi-Fi the page loads only the two calls the hostname field depends on.
    store.setHardwareCapabilities(deckCapabilities({ wifiSupported: false, ethernetSupported: true }));
    registerMocks(pb.services.NetworkService, {
        getNetworkInfo: () =>
            pb.create(pb.NetworkInfoResponseSchema, { hostname: deviceHostname, ipAddress: '10.0.0.7' }),
        getNetworkConfig: () => pb.create(pb.NetworkConfigSchema),
        setHostname,
    });
    return render(
        <HelmetProvider>
            <RawIntlProvider value={intlFor({ tag: 'en' })}>
                <MemoryRouter>
                    <Network />
                    <Toaster />
                </MemoryRouter>
            </RawIntlProvider>
        </HelmetProvider>,
    );
}

const hostnameInput = async (value: string): Promise<HTMLInputElement> => {
    const input = await screen.findByDisplayValue(value);
    return input as HTMLInputElement;
};
const saveButton = (): HTMLElement | null => screen.queryByRole('button', { name: 'Save hostname' });

afterEach(() => {
    cleanup();
    mocks.clear();
    store.setHardwareCapabilities(null);
    deviceHostname = 'braiins-deck';
    renameRequests = [];
});

describe('Network hostname (BDK-1102)', () => {
    test('the field shows the current name and the address it is reachable at', async () => {
        renderPage();

        await hostnameInput('braiins-deck');

        expect(screen.getByText(/http:\/\/braiins-deck\.local/)).toBeTruthy();
        expect(saveButton()).toBeNull();
    });

    test('saving a new name sends it to the device and shows the new address', async () => {
        renderPage();
        const input = await hostnameInput('braiins-deck');

        fireEvent.change(input, { target: { value: 'kitchen-deck' } });
        const save = saveButton();
        expect(save).not.toBeNull();
        fireEvent.click(save as HTMLElement);

        await waitFor(() => expect(renameRequests).toEqual(['kitchen-deck']));
        // The reload after the save reads the name back from the device, so the
        // address the user is told to use is the one the device really answers to.
        await waitFor(() => expect(saveButton()).toBeNull());
        expect(screen.getByText(/http:\/\/kitchen-deck\.local/)).toBeTruthy();
    });

    test('a name the device refuses is reported at the field and stays editable', async () => {
        renderPage(() => {
            const detail = create(BadRequestSchema, {
                fieldViolations: [{ field: 'hostname', description: 'hostname must not be empty' }],
            });
            throw new ConnectError('Bad request', Code.InvalidArgument, undefined, [
                { desc: BadRequestSchema, value: detail },
            ]);
        });
        const input = await hostnameInput('braiins-deck');

        fireEvent.change(input, { target: { value: 'kitchen deck' } });
        fireEvent.click(saveButton() as HTMLElement);

        expect(await screen.findByText(/must not be empty/)).toBeTruthy();
        // The refused text stays in the field for the user to correct, with the save still on offer.
        expect(input.value).toBe('kitchen deck');
        expect(saveButton()).not.toBeNull();
    });
});
