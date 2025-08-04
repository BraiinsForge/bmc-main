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
import PageDisplayCombined from './pages/workspace/Display/DisplayCombined.tsx';

import PageAlarms from './pages/workspace/Alarms';
import PagePriceAlerts from './pages/workspace/PriceAlerts';
import PageNotifications from './pages/workspace/Notifications';
import PageNetwork from './pages/workspace/Network';
import PageApi from './pages/workspace/Api';
import PageBuyButton from './pages/workspace/BuyButton';

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
                    { path: URLS.pages.api, Component: PageApi },
                    { path: URLS.pages.buyButton, Component: PageBuyButton },
                ],
            },
        ],
    } satisfies RouteObject,

    // 404
    { path: '*', Component: PageNotFound },
]);
