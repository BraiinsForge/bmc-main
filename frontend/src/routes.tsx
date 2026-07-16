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

import { createBrowserRouter, type RouteObject } from 'react-router';

import { URLS } from './constants';

import Root from './pages/Root';
import ContainerAuth from './pages/auth/Container';
import ContainerWorkspace from './pages/workspace/Container';

import PageInitSetup from './pages/init/InitSetup';
import PageNotFound from './pages/404';

import PageLogin from './pages/auth/Login';
import PageSettings from './pages/workspace/Settings';

import PageDisplayList from './pages/workspace/Display/DisplayList';
import PageDisplayCombined from './pages/workspace/Display/DisplayCombined';

import PageAlarms from './pages/workspace/Alarms';
import PagePriceAlerts from './pages/workspace/PriceAlerts';
import PageNotifications from './pages/workspace/Notifications';
import PageNetwork from './pages/workspace/Network';
import PageAccounts from './pages/workspace/Accounts';

export default createBrowserRouter([
    // Initial setup (right after wifi is configured)
    // Has to be a root level route because the Root page wrapper
    // redirects to login if valid session is not found.
    { path: URLS.pages.initSetup, Component: PageInitSetup },

    {
        path: '/',
        Component: Root,
        children: [
            // Auth
            {
                Component: ContainerAuth,
                children: [
                    // Login
                    { path: URLS.auth.login, Component: PageLogin },
                ],
            },

            // Workspace
            {
                Component: ContainerWorkspace,
                children: [
                    { path: URLS.pages.settings, Component: PageSettings },

                    { path: URLS.pages.display.list, Component: PageDisplayList },
                    { path: URLS.pages.display.combined.path, Component: PageDisplayCombined },

                    { path: URLS.pages.alarms, Component: PageAlarms },
                    { path: URLS.pages.priceAlerts, Component: PagePriceAlerts },
                    { path: URLS.pages.notifications, Component: PageNotifications },
                    { path: URLS.pages.network, Component: PageNetwork },
                    { path: URLS.pages.accounts, Component: PageAccounts },
                ],
            },
        ],
    } satisfies RouteObject,

    // 404
    { path: '*', Component: PageNotFound },
]);
