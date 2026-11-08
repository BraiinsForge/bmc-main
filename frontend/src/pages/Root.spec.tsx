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
import { cleanup, render, screen, within } from '@testing-library/react/pure';
import { use, useEffect } from 'react';
import { RawIntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import AppContext from '@/context';
import { pseudoIntl } from '@/mocks/intl';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import Root from './Root';

rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

afterEach(() => {
    cleanup();
    store.setHardwareCapabilities(null);
});

function AskWithoutLabels() {
    const { confirm } = use(AppContext);
    useEffect(() => {
        void confirm({ title: 'Delete the alarm?', message: 'It will not ring again.' });
    }, [confirm]);
    return null;
}

describe('Root confirmation', () => {
    test('a confirmation without labels words its buttons through the catalog', async () => {
        store.setHardwareCapabilities(deckCapabilities());
        render(
            <RawIntlProvider value={pseudoIntl}>
                <MemoryRouter>
                    <Routes>
                        <Route element={<Root />}>
                            <Route path="*" element={<AskWithoutLabels />} />
                        </Route>
                    </Routes>
                </MemoryRouter>
            </RawIntlProvider>,
        );

        const dialog = await screen.findByRole('dialog');
        expect(within(dialog).queryByRole('button', { name: 'Confirm' })).toBeNull();
        expect(within(dialog).queryByRole('button', { name: 'Cancel' })).toBeNull();
        const respelled = within(dialog).getAllByRole('button', { name: /^\[.+\]$/ });
        expect(respelled.length, 'the pseudo-locale brackets what it respells').toBeGreaterThanOrEqual(2);
    });
});
