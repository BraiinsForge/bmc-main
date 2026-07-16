// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { getID } from '@/lib/form';
import { formatAlarmTime } from '@/lib/time';

// App
import * as pb from '@/proto';

// Components
import { Button, ButtonGroup, DataTable, type DataTableHeader, type DataTableRow } from '@/components';
import { Toggle } from '@carbon/react';
import { TrashCan as IconDelete, Time as IconClock } from '@carbon/react/icons';

// Styles
import css from './AlarmsTable.scss';

const $ = getID('alarms').get;
type TableCol = 'state' | 'time' | 'label' | 'repeat' | 'sound' | 'snooze' | 'actions';

export interface AlarmsTableProps {
    isLoading: boolean;
    data: pb.Alarm[];
    timeFormat: pb.TimeFormat;
    onEdit(id: pb.Alarm['id']): void;
    onToggle(id: pb.Alarm['id'], checked: boolean): void;
    onDelete(id: pb.Alarm['id']): void;
}
interface Props extends AlarmsTableProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    #getHeaders = (): Array<DataTableHeader<TableCol>> => {
        const { formatMessage } = this.props.intl;

        return [
            {
                key: 'state',
                header: '',
                align: 'start',
                maxWidth: 90,
            },
            {
                key: 'time',
                header: formatMessage({ defaultMessage: 'Time' }),
                align: 'end',
            },
            {
                key: 'label',
                header: formatMessage({ defaultMessage: 'Label' }),
                align: 'start',
            },
            {
                key: 'repeat',
                header: formatMessage({ defaultMessage: 'Repeat' }),
                align: 'start',
            },
            {
                key: 'sound',
                header: formatMessage({ defaultMessage: 'Alarm Sound' }),
                align: 'start',
            },
            {
                key: 'snooze',
                header: formatMessage({ defaultMessage: 'Snooze' }),
                align: 'start',
            },
            {
                key: 'actions',
                header: formatMessage({ defaultMessage: 'Actions' }),
                align: 'start',
                // Extra width helps with preventing tooltip caused overflow
                minWidth: 100,
            },
        ];
    };
    #getRows = (): Array<DataTableRow<TableCol>> => {
        const { data, timeFormat, onToggle, onEdit, onDelete, intl } = this.props;
        const { formatMessage } = intl;

        return data.map<DataTableRow<TableCol>>(alarm => {
            const { id, time, name, enabled, snoozeOptions, repeat, sound } = alarm;

            return {
                id,
                cells: {
                    state: (
                        <Toggle
                            id={$('state', id)}
                            toggled={enabled}
                            labelA={formatMessage({ defaultMessage: 'Off' })}
                            labelB={formatMessage({ defaultMessage: 'On' })}
                            onToggle={checked => onToggle(id, checked)}
                        />
                    ),
                    time: <strong children={formatAlarmTime(time, timeFormat)} />,
                    label: <span children={name} className={css.labelWrapper} />,
                    repeat: pb.weekdayListToString(intl, repeat),
                    sound: sound?.name ?? '--',
                    snooze: pb.alarmSnoozeOptionsToString(intl, snoozeOptions),
                    actions: (
                        <ButtonGroup spaced>
                            <Button
                                id={$('edit', id)}
                                size="sm"
                                kind="primary"
                                children={formatMessage({ defaultMessage: 'Edit' })}
                                onClick={() => onEdit(id)}
                            />
                            <Button
                                id={$('delete', id)}
                                size="sm"
                                kind="secondary"
                                hasIconOnly
                                icon={IconDelete}
                                title={formatMessage({ defaultMessage: 'Delete' })}
                                tooltipPosition="top"
                                tooltipAlignment="end"
                                onClick={() => onDelete(id)}
                            />
                        </ButtonGroup>
                    ),
                },
            };
        });
    };

    render() {
        const { isLoading } = this.props;
        const { formatMessage } = this.props.intl;

        return (
            <div className={css.root}>
                <DataTable
                    className={css.table}
                    headers={this.#getHeaders()}
                    rows={this.#getRows()}
                    placeholder={{
                        icon: IconClock,
                        title: formatMessage({ defaultMessage: 'No Alarms Yet' }),
                        message: formatMessage({ defaultMessage: 'Create your first alarm to get started' }),
                    }}
                    skeletonRowsCount={5}
                    isLoading={isLoading}
                />
            </div>
        );
    }
}

export function AlarmsTable(props: AlarmsTableProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
