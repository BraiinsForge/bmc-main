import { StrictMode } from 'react';
import { IntlProvider } from 'react-intl';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from 'react-router';
import { HelmetProvider, Helmet } from '@dr.pogodin/react-helmet';

import router from '@/routes';

const rootEl = document.getElementById('root');
if (rootEl) {
    const root = createRoot(rootEl);
    root.render(
        <StrictMode>
            <HelmetProvider>
                <Helmet defaultTitle="Braiins BMC-100" titleTemplate="%s | Braiins BMC-100" />
                <IntlProvider locale="en">
                    <RouterProvider router={router} />
                </IntlProvider>
            </HelmetProvider>
        </StrictMode>,
    );
}
