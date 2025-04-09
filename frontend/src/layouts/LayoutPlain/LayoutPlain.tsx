import { Fragment, Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import { Braiins } from '@/res/svg';
import { Content, Header, HeaderName, SkipToContent } from '@carbon/react';

import css from './LayoutPlain.scss';

export interface LayoutPlainProps {
    children: ReactNode;
}
interface Props extends LayoutPlainProps {
    intl: IntlShape;
}

class Base extends Component<Props> {
    #txt = {
        name: this.props.intl.formatMessage({ defaultMessage: 'BMC 100 - Braiins Mining Clock' }),
        documentation: this.props.intl.formatMessage({ defaultMessage: 'Documentation' }),
    };

    render() {
        const { children } = this.props;

        return (
            <Fragment>
                <Header aria-label={this.#txt.name}>
                    <SkipToContent />
                    <HeaderName href="/" prefix="" className={css.headerName}>
                        <Braiins width={114} />
                        <span role="presentation" children={this.#txt.name} />
                    </HeaderName>
                </Header>
                <Content id="main-content" className={css.content} children={children} />
            </Fragment>
        );
    }
}

export function LayoutPlain(props: LayoutPlainProps) {
    const intl = useIntl();
    return <Base {...props} intl={intl} />;
}
