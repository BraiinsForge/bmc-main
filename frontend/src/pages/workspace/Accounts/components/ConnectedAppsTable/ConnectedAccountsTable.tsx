import { Component, useState, useCallback, type RefObject } from 'react';
import { Sized } from '@/lib/react';
import { useIntl, type IntlShape } from 'react-intl';

// APP
import * as pb from '@/proto';
import { getID } from '../const';

// Components
import {
    DataTable,
    type DataTableHeader,
    type DataTableRow,
    AccountIcon,
    Datetime,
    Button,
    ButtonGroup,
} from '@/components';
import { TrashCan as IconDelete, Edit as IconEdit, View as IconShow, ViewOff as IconHide } from '@carbon/react/icons';

// Styles
import css from './ConnectedAppsTable.scss';

export interface ConnectedAccountsTableProps {
    accounts: pb.Account[];
    onEdit(acc: pb.Account): void;
    onDelete(acc: pb.Account): void;
}
interface Props extends ConnectedAccountsTableProps {
    intl: IntlShape;
    tableRef?: RefObject<null | HTMLDivElement>;
    layout: {
        xs: boolean;
        sm: boolean;
    };
}

type TableCol = 'type' | 'name' | 'apiKey' | 'createdAt' | 'actions';
const NA = <span children="N/A" />;
const $ = getID('accounts-table').get;

class View extends Component<Props> {
    get #headers(): Array<DataTableHeader<TableCol>> {
        const {
            intl: { formatMessage },
            layout,
        } = this.props;

        const $type: DataTableHeader<TableCol> = {
            key: 'type',
            header: formatMessage({ defaultMessage: 'Type' }),
            align: 'start',
        };
        const $name: DataTableHeader<TableCol> = {
            key: 'name',
            header: formatMessage({ defaultMessage: 'Account' }),
            align: 'start',
        };
        const $apiKey: DataTableHeader<TableCol> = {
            key: 'apiKey',
            header: formatMessage({ defaultMessage: 'API Key' }),
            align: 'start',
        };
        const $createdAt: DataTableHeader<TableCol> = {
            key: 'createdAt',
            header: formatMessage({ defaultMessage: 'Created At' }),
            align: 'end',
        };
        const $actions: DataTableHeader<TableCol> = {
            key: 'actions',
            header: formatMessage({ defaultMessage: 'Action' }),
            align: 'end',
        };

        if (layout.xs) return [$type, $actions];
        if (layout.sm) return [$type, $name, $actions];

        return [$type, $name, $apiKey, $createdAt, $actions];
    }
    get #rows(): Array<DataTableRow<TableCol>> {
        const { accounts, onEdit, onDelete, layout, intl } = this.props;

        return accounts.map((x, index) => {
            const editLabel: string = intl.formatMessage({ defaultMessage: 'Edit' });
            const editProps = {
                id: $('edit', index + 1),
                size: 'sm',
                kind: 'primary',
                tooltipPosition: 'left',
                onClick: () => onEdit(x),
            } as const;
            const editButton: ReactElement = layout.sm ? (
                <Button {...editProps} key="edit" title={editLabel} icon={IconEdit} hasIconOnly />
            ) : (
                <Button {...editProps} key="edit" children={editLabel} />
            );

            return {
                id: x.id,
                cells: {
                    type: (
                        <div className={css.typeColContent}>
                            <AccountIcon size={24} type={x.accountType} />
                            <span children={pb.accountTypeToString(intl, x.accountType)} />
                        </div>
                    ),
                    name: x.accountName,
                    apiKey: <ApiKey id={$('api-key', index)} value={x.authentication?.value?.value} />,
                    createdAt: <Datetime value={x.createdAt} format="%d.%m.%Y %H:%M" />,
                    actions: (
                        <ButtonGroup spaced>
                            {editButton}
                            <Button
                                id={$('delete', index + 1)}
                                size="sm"
                                kind="secondary"
                                icon={IconDelete}
                                hasIconOnly
                                title={intl.formatMessage({ defaultMessage: 'Delete' })}
                                tooltipPosition="left"
                                onClick={() => onDelete(x)}
                            />
                        </ButtonGroup>
                    ),
                },
            };
        });
    }

    render() {
        const { tableRef } = this.props;

        return (
            <div className={css.root} ref={tableRef}>
                <DataTable headers={this.#headers} rows={this.#rows} />
            </div>
        );
    }
}
export function ConnectedAccountsTable(props: ConnectedAccountsTableProps) {
    const intl = useIntl();
    return (
        <Sized<HTMLDivElement>
            render={(ref, size) => {
                const width = size?.width ?? Number.MAX_SAFE_INTEGER;
                const sm: boolean = width <= 700;
                const xs: boolean = width <= 500;

                return <View {...props} tableRef={ref} intl={intl} layout={{ sm, xs }} />;
            }}
        />
    );
}

interface ApiKeyProps {
    id: string;
    value: Maybe<string>;
}
function ApiKey(props: ApiKeyProps) {
    const { id, value } = props;

    const { formatMessage } = useIntl();
    const [shown, setShown] = useState<boolean>(false);
    const toggleShown = useCallback(() => setShown(x => !x), []);

    if (!value) return NA;
    return (
        <div className={css.apiToken}>
            <span children={shown ? value : '****************'} />
            <Button
                id={id}
                size="sm"
                kind="ghost"
                icon={shown ? IconHide : IconShow}
                hasIconOnly
                title={shown ? formatMessage({ defaultMessage: 'Hide' }) : formatMessage({ defaultMessage: 'Show' })}
                onClick={toggleShown}
            />
        </div>
    );
}
