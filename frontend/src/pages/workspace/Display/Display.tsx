import { Component } from 'react';
import { Helmet } from '@dr.pogodin/react-helmet';
import { useIntl, type IntlShape } from 'react-intl';

import css from './Display.scss';

import AppContext, { type AppContextType } from '@/context';
import { Button, ButtonGroup } from '@/components';
import { Add } from '@carbon/react/icons';

interface Props {
    intl: IntlShape;
}

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

    #title = this.props.intl.formatMessage({ defaultMessage: 'Display Scenes' });

    #onSceneAdd = () => this.context.notify('error', 'Not implemented!');

    render() {
        const { formatMessage } = this.props.intl;

        return (
            <div>
                <Helmet title={this.#title} />
                <header className={css.header}>
                    <h1 className={css.title} children={this.#title} />
                    <ButtonGroup spaced>
                        <Button
                            kind="primary"
                            onClick={this.#onSceneAdd}
                            icon={Add}
                            children={formatMessage({ defaultMessage: 'Add New Scene' })}
                        />
                    </ButtonGroup>
                </header>
            </div>
        );
    }
}

export default function DisplayPage() {
    const intl = useIntl();
    return <View intl={intl} />;
}
