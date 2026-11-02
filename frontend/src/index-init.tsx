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

import 'core-js/actual';

import { StrictMode, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import { readSystem } from '@/lib/system';
import { store } from '@/store';
import { BootError } from '@/components';
import { LocaleProvider } from '@/components/LocaleProvider';
import '@/lib/carbonFlags';
import App from './pages/init/InitWifi';

function boot(rootEl: HTMLElement): void {
    let tree: ReactNode;

    try {
        // Before the first render, so the page never starts in another language.
        store.boot();
        tree = <App capabilities={readSystem().capabilities} />;
    } catch (error) {
        tree = <BootError error={error} />;
    }

    createRoot(rootEl).render(
        <StrictMode>
            <HelmetProvider>
                <LocaleProvider children={tree} />
            </HelmetProvider>
        </StrictMode>,
    );
}

const rootEl = document.getElementById('root');
if (rootEl) boot(rootEl);
