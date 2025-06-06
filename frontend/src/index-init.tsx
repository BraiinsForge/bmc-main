import 'core-js/actual';

import { StrictMode, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { createIntl, createIntlCache, RawIntlProvider } from 'react-intl';

import App from './pages/init/InitWifi';

const noop = () => {};
const empty = Object.freeze({});
const intlCache = createIntlCache();
const intlObject = createIntl(
    { locale: 'en', timeZone: 'UTC', messages: empty, onWarn: noop, onError: noop },
    intlCache,
);
export function IntlProvider(props: { children: ReactNode }) {
    return <RawIntlProvider value={intlObject} children={props.children} />;
}

const rootEl = document.getElementById('root');
if (rootEl) {
    const root = createRoot(rootEl);
    root.render(
        <StrictMode>
            <IntlProvider>
                <App />
            </IntlProvider>
        </StrictMode>,
    );
}
