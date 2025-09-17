import 'core-js/actual';
import '@/lib/polyfill';

import { isEqual } from 'es-toolkit';
import { Component, type ReactNode, StrictMode } from 'react';

// Hoc
import { IntlProvider } from 'react-intl';
import { MemoryRouter } from 'react-router';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

// Styles
import '@/styles/carbon/carbon.global.scss';
import './Container.global.scss';

const THEME = require('@/styles/theme').default;
const noop = () => {};

type Props = {
    story(): ReactNode;
    className?: string;
    theme: null | keyof typeof THEME;
    i18n: null | {
        locale: string;
        name: string;
        bidi: boolean;
        formats: Record<string, string>;
        messages: Record<string, string>;
    };
};

type SetDomAttrsData = Pick<Props, 'theme' | 'i18n'>;
function setDomAttributes(props: SetDomAttrsData): void {
    const { theme, i18n } = props;
    const d = document;

    // Set body style
    d.body.style.padding = '';
    d.body.style.margin = '';
    d.body.style.width = '100%';
    d.body.style.maxWidth = '100%';
    d.body.style.height = '100%';
    d.body.style.maxHeight = '100%';

    if (theme) {
        const value = theme in THEME ? THEME[theme] : THEME.dark;
        window.document.documentElement.setAttribute('class', value);
    }

    const doc = document.documentElement;
    if (i18n?.bidi) doc.setAttribute('dir', 'rtl');
    else doc.removeAttribute('dir');
}

export default class Container extends Component<Props> {
    constructor(props: Props) {
        super(props);

        // This has to be set as soon as possible.
        // The "componentDidMount" method is too late,
        // since then a lot of code that need this set
        // might have already been executed.
        setDomAttributes(props);
    }

    componentDidUpdate(prevProps: Props) {
        const dataPrev: SetDomAttrsData = { i18n: prevProps.i18n, theme: prevProps.theme };
        const dataCurr: SetDomAttrsData = { i18n: this.props.i18n, theme: this.props.theme };
        if (!isEqual(dataPrev, dataCurr)) setDomAttributes(dataCurr);
    }

    render() {
        const { story, className, i18n } = this.props;

        const locale = i18n?.locale ?? 'en';
        const msg = i18n?.messages || {};

        return (
            <StrictMode>
                <HelmetProvider>
                    <IntlProvider key={locale} defaultLocale={locale} locale={locale} messages={msg} onError={noop}>
                        <MemoryRouter>
                            <div role="main" className={className}>
                                {story()}
                            </div>
                        </MemoryRouter>
                    </IntlProvider>
                </HelmetProvider>
            </StrictMode>
        );
    }
}
