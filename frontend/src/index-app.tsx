// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { StrictMode, type ReactNode } from 'react';
import { IntlProvider } from 'react-intl';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from 'react-router';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import router from '@/routes';
import { store } from '@/store';
import { BootError } from '@/components';
import { AppHead } from '@/components/AppHead';
import '@/styles/carbon/carbon.global.scss';

function boot(rootEl: HTMLElement): void {
    const root = createRoot(rootEl);
    let tree: ReactNode;
    try {
        store.boot();
        tree = (
            <HelmetProvider>
                <AppHead />
                <RouterProvider router={router} />
            </HelmetProvider>
        );
    } catch (error) {
        tree = <BootError error={error} />;
    }
    root.render(
        <StrictMode>
            <IntlProvider locale="en">{tree}</IntlProvider>
        </StrictMode>,
    );
}

const rootEl = document.getElementById('root');
if (rootEl) boot(rootEl);
