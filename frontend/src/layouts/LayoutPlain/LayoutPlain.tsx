import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Content } from '@carbon/react';

import css from './LayoutPlain.scss';

export interface LayoutPlainProps {
    children: ReactNode;
}
interface Props extends LayoutPlainProps {
    intl: IntlShape;
}

class Base extends Component<Props> {
    render() {
        const { children } = this.props;

        return <Content id="main-content" className={css.content} children={children} />;
    }
}

export function LayoutPlain(props: LayoutPlainProps) {
    const intl = useIntl();
    return <Base {...props} intl={intl} />;
}
