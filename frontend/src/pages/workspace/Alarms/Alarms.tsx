import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Helmet } from '@dr.pogodin/react-helmet';

// Components
import { Button, DataTable, type DataTableHeader, type DataTableRow } from '@/components';
import { Add as IconAdd } from '@carbon/react/icons';

// Styles
import css from './Alarms.scss';

type TableCol = 'state' | 'time' | 'label' | 'repeat' | 'sound' | 'snooze' | 'actions';

interface Props {
    intl: IntlShape;
}

export class View extends Component<Props> {
    #getTableHeaders = (): Array<DataTableHeader<TableCol>> => {
        const { formatMessage } = this.props.intl;

        return [
            { key: 'state', header: '', align: 'start', maxWidth: 90 },
            { key: 'time', header: formatMessage({ defaultMessage: 'Time' }), align: 'end' },
            { key: 'label', header: formatMessage({ defaultMessage: 'Label' }), align: 'start' },
            { key: 'repeat', header: formatMessage({ defaultMessage: 'Repeat' }), align: 'start' },
            { key: 'sound', header: formatMessage({ defaultMessage: 'Alarm Sound' }), align: 'start' },
            { key: 'snooze', header: formatMessage({ defaultMessage: 'Snooze' }), align: 'start' },
            { key: 'actions', header: formatMessage({ defaultMessage: 'Actions' }), align: 'start' },
        ];
    };
    #getTableRows = (): Array<DataTableRow<TableCol>> => {
        return [];
    };

    render() {
        const { formatMessage } = this.props.intl;
        const title = formatMessage({ defaultMessage: 'Alarms' });

        return (
            <div className={css.root}>
                <Helmet title={title} />
                <div className={css.header}>
                    <div className={css.labels}>
                        <h1 className={css.title} children={title} />
                        <div
                            className={css.description}
                            children={formatMessage({
                                defaultMessage:
                                    'Let’s make sure your alarm wakes you just right – not too loud, not too chill."',
                            })}
                        />
                    </div>
                    <div className={css.actions}>
                        <Button
                            disabled
                            renderIcon={IconAdd}
                            children={formatMessage({ defaultMessage: 'Add New Alarm' })}
                        />
                    </div>
                </div>

                <div className={css.content}>
                    <DataTable
                        headers={this.#getTableHeaders()}
                        rows={this.#getTableRows()}
                        placeholder={{
                            title: formatMessage({ defaultMessage: 'No alarms found' }),
                            message: formatMessage({ defaultMessage: 'Create your first alarm to get started' }),
                        }}
                        skeletonRowsCount={5}
                    />
                </div>
            </div>
        );
    }
}

export default function AlarmsPage() {
    const intl = useIntl();
    return <View intl={intl} />;
}
