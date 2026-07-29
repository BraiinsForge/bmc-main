// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { Component, type RefObject } from 'react';
import { Sized } from '@/lib/react';
import { useIntl, type IntlShape } from 'react-intl';

// APP
import type * as pb from '@/proto';
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
import { TrashCan as IconDelete, Edit as IconEdit } from '@carbon/react/icons';

// Styles
import css from './ConnectedAppsTable.scss';

export interface ConnectedAccountsTableProps {
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;
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

type TableCol = 'type' | 'name' | 'createdAt' | 'actions';
const $ = getID('accounts-table').get;

class View extends Component<Props> {
    #type(typeId: string): pb.CredentialType | undefined {
        return this.props.credentialTypes.get(typeId);
    }

    #typeName(typeId: string): string {
        return this.#type(typeId)?.name ?? typeId;
    }

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

        return [$type, $name, $createdAt, $actions];
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
                            <AccountIcon size={24} icon={this.#type(x.typeId)?.icon} />
                            <span children={this.#typeName(x.typeId)} />
                        </div>
                    ),
                    name: x.name,
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
