// Copyright (C) 2026  Braiins Systems s.r.o.
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
import { cleanup, render, waitFor } from '@testing-library/react/pure';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import type { Brand } from '@/lib/brand';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import { store } from '@/store';
import { AppHead } from './AppHead';

const BOS_BRAND: Brand = { name: 'Braiins OS', logo: { header: null }, links: { products: null } };
const NO_BRAND: Brand = { name: null, logo: { header: null }, links: { products: null } };

afterEach(() => {
    cleanup();
    store.setBrand(null);
});

describe('AppHead', () => {
    test('a standalone Deck keeps its own title', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: false }));
        render(
            <HelmetProvider>
                <AppHead />
            </HelmetProvider>,
        );

        await waitFor(() => expect(document.title).toBe('Braiins DECK'));
    });

    test('a boser-managed device takes the title from the brand', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: true }));
        store.setBrand(BOS_BRAND);
        render(
            <HelmetProvider>
                <AppHead />
            </HelmetProvider>,
        );

        await waitFor(() => expect(document.title).toBe('Braiins OS'));
    });

    test('a brand without a usable name keeps the Deck title', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: true }));
        store.setBrand(NO_BRAND);
        render(
            <HelmetProvider>
                <AppHead />
            </HelmetProvider>,
        );

        await waitFor(() => expect(document.title).toBe('Braiins DECK'));
    });
});
