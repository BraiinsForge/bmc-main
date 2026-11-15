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
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider, RawIntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import { pseudoIntl } from '@/mocks/intl';
import { proto } from '@/mocks/proto';
import { store } from '@/store';
import LoginPage from './Login';

afterEach(() => {
    cleanup();
    rstest.restoreAllMocks();
});

describe('LoginPage', () => {
    test("labels the password field in the page's language", () => {
        render(
            <HelmetProvider>
                <RawIntlProvider value={pseudoIntl}>
                    <LoginPage />
                </RawIntlProvider>
            </HelmetProvider>,
        );

        expect(screen.getByLabelText('[Ƥȧȧşşẇǿǿřḓ]')).toBeTruthy();
    });

    test('a login that loses the device reports it in the page’s words', async () => {
        rstest.spyOn(store, 'login').mockRejectedValue(proto.lostConnection());
        render(
            <HelmetProvider>
                <IntlProvider locale="en">
                    <LoginPage />
                </IntlProvider>
            </HelmetProvider>,
        );

        fireEvent.click(screen.getByRole('button', { name: 'Login' }));

        expect(await screen.findByText('Failed to log in!')).toBeTruthy();
        expect(screen.queryByText(/Failed to fetch/)).toBeNull();
    });
});
