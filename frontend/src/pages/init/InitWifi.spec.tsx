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

import { afterEach, describe, expect, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { Code, ConnectError } from '@connectrpc/connect';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import InitWifi from './InitWifi';

function scanFailsWith(error: ConnectError): void {
    const methods: Partial<ServiceMocks<typeof pb.services.InitialSetupService>> = {
        scanWifi: () => {
            throw error;
        },
    };
    mocks.service(pb.services.InitialSetupService, methods as ServiceMocks<typeof pb.services.InitialSetupService>);
}

function openTheWifiStep(): void {
    render(
        <IntlProvider locale="en">
            <InitWifi capabilities={deckCapabilities()} />
        </IntlProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
}

afterEach(() => {
    cleanup();
    mocks.clear();
});

describe('InitWifi', () => {
    test('a device that cannot be reached is reported in the page’s words', async () => {
        scanFailsWith(new ConnectError('Failed to fetch', Code.Unknown));
        openTheWifiStep();

        expect(await screen.findByText('Failed to load Wi-Fi networks!')).toBeTruthy();
        expect(screen.queryByText('Failed to fetch')).toBeNull();
    });

    test('a device that refuses is reported in its own words', async () => {
        scanFailsWith(new ConnectError('Not now.', Code.FailedPrecondition));
        openTheWifiStep();

        expect(await screen.findByText('Not now.')).toBeTruthy();
    });
});
