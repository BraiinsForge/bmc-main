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

import { afterEach, describe, test, expect, rstest } from '@rstest/core';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { MemoryRouter } from 'react-router';
import { IntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import InitSetup from './InitSetup';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { timezones } from '@/mocks';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';

// `mocks.service` wants every method typed; at runtime it only registers what we
// pass. This lets us register a typed subset.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

const miner = deckCapabilities({ miningSupported: true, ethernetSupported: true, productName: 'Braiins Mini Miner' });
const deck = deckCapabilities();

const POOL_URL = 'stratum+tcp://solo.stratum.braiins.com:3333';
const HOSTNAME = 'miner-01';
const settingsData = pb.create(pb.SettingsDataResponseSchema, {
    timezones,
    timezoneId: timezones[0]?.id ?? 'Europe/Prague',
    timeFormat: pb.TimeFormat.TIME_FORMAT_24_HOUR,
    dateFormat: pb.DateFormat.DD_MM_YYYY_DOT,
    numberFormat: pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL,
    temperatureUnit: pb.TemperatureUnit.CELSIUS,
    unitSystem: pb.UnitSystem.METRIC,
    hostname: HOSTNAME,
    pool: pb.create(pb.PoolConfigSchema, { url: POOL_URL, user: '' }),
    network: pb.create(pb.NetworkConfigSchema, { protocol: { case: 'dhcp', value: {} } }),
});

type SetupDeviceMock = ServiceMocks<typeof pb.services.InitialSetupService>['setupDevice'];

function installMocks(capabilities: pb.HardwareCapabilities, setupDevice: SetupDeviceMock): void {
    mocks.clear();
    registerMocks(pb.services.HardwareService, { getHardwareCapabilities: () => capabilities });
    registerMocks(pb.services.InitialSetupService, {
        getSettingsData: () => settingsData,
        setupDevice,
    });
}

// jsdom won't let us spy on location.replace directly; swap the whole location.
const REAL_LOCATION = window.location;
function stubReplace(): ReturnType<typeof rstest.fn> {
    const replace = rstest.fn();
    Object.defineProperty(window, 'location', {
        configurable: true,
        value: Object.assign(new URL(window.location.href), { replace }),
    });
    return replace;
}

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter>
                    <InitSetup />
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );
}

afterEach(() => {
    cleanup();
    mocks.clear();
    Object.defineProperty(window, 'location', { configurable: true, value: REAL_LOCATION });
});

describe('InitSetup', () => {
    test('a miner gets the pool and network form, prefilled from the backend', async () => {
        installMocks(
            miner,
            rstest.fn(() => ({})),
        );
        renderPage();

        await screen.findByText('Mining Pool');
        expect(screen.getByText('Ethernet Network')).toBeTruthy();
        expect(screen.getByDisplayValue(POOL_URL)).toBeTruthy();
        expect(screen.getByDisplayValue(HOSTNAME)).toBeTruthy();
    });

    test('a display device gets the localization form only', async () => {
        installMocks(
            deck,
            rstest.fn(() => ({})),
        );
        renderPage();

        await screen.findByText('Time, Date and Regional Settings');
        expect(screen.queryByText('Mining Pool')).toBeNull();
    });

    test('saving a miner sends the pool and network and leaves for the site root', async () => {
        let received: pb.SettingsRequest | undefined;
        const setupDevice: SetupDeviceMock = ({ req }) => {
            received = req;
            return {};
        };
        installMocks(miner, setupDevice);
        const replace = stubReplace();
        renderPage();

        await screen.findByText('Mining Pool');
        fireEvent.click(screen.getByRole('button', { name: 'Save and Continue' }));

        await waitFor(() => expect(received).toBeDefined());
        expect(received?.pool?.url).toBe(POOL_URL);
        expect(received?.hostname).toBe(HOSTNAME);
        expect(received?.network?.protocol.case).toBe('dhcp');
        await waitFor(() => expect(replace).toHaveBeenCalledWith('/'));
    });
});
