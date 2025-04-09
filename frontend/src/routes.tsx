import { createBrowserRouter, type RouteObject } from 'react-router';

import { URLS } from './constants';

import Root from './pages/Root';
import ContainerAuth from './pages/auth/Container';
import ContainerWorkspace from './pages/workspace/Container';

import PageLogin from './pages/auth/Login';

import PageSettings from './pages/workspace/Settings';
import PageDisplay from './pages/workspace/Display';
import PageAlarms from './pages/workspace/Alarms';
import PagePriceAlerts from './pages/workspace/PriceAlerts';
import PageNotifications from './pages/workspace/Notifications';
import PageNetwork from './pages/workspace/Network';
import PageApi from './pages/workspace/Api';
import PageBuyButton from './pages/workspace/BuyButton';

export default createBrowserRouter([
    {
        path: '/',
        Component: Root,
        children: [
            // Auth
            {
                Component: ContainerAuth,
                children: [{ path: URLS.auth.login, Component: PageLogin }],
            },

            // Workspace
            {
                Component: ContainerWorkspace,
                children: [
                    { path: URLS.pages.settings, Component: PageSettings },
                    { path: URLS.pages.display, Component: PageDisplay },
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
]);
