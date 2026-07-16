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

import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import * as get from '@/mocks';

import { AlarmsTable as Component, type AlarmsTableProps } from './AlarmsTable';

// HH:MM, Range 00:00 - 23:59
function getTime(hour: [min: number, max: number] = [0, 23], minute: [min: number, max: number] = [0, 59]): string {
    const h: number = get.number(hour[0], hour[1], false);
    const hh: string = String(h).padStart(2, '0');

    const m: number = get.number(minute[0], minute[1]);
    const mm: string = String(m).padStart(2, '0');

    return `${hh}:${mm}`;
}
function getRepeat(count: get.LengthRange): pb.Weekday[] {
    return get.randomSlice<pb.Weekday>(
        [
            pb.Weekday.MONDAY,
            pb.Weekday.TUESDAY,
            pb.Weekday.WEDNESDAY,
            pb.Weekday.THURSDAY,
            pb.Weekday.FRIDAY,
            pb.Weekday.SATURDAY,
            pb.Weekday.SUNDAY,
        ],
        count,
    );
}
function getSnooze(): pb.SnoozeOptionsWrapper {
    return {
        $typeName: 'braiins.bmc.web.SnoozeOptionsWrapper',
        kind: get.randomItem([
            {
                case: 'snooze' as const,
                value: pb.create(pb.SnoozeOptionsSchema, {
                    $typeName: 'braiins.bmc.web.SnoozeOptions',
                    duration: get.proto.randomEnumItem(pb.SnoozeDuration),
                    limit: get.proto.randomEnumItem(pb.SnoozeLimit),
                }),
            },
            {
                case: 'off' as const,
                value: pb.create(pb.OffSchema),
            },
        ]),
    };
}
function getSound() {
    return pb.create(pb.SoundInfoSchema, {
        id: get.uuid(),
        name: get.hostname(2, ''),
    });
}

export default {
    title: 'Alarms/components/AlarmsTable',
    component: Component,
    args: {
        onEdit: action('onEdit'),
        onDelete: action('onDelete'),
        onToggle: action('onToggle'),
        data: [
            pb.create(pb.AlarmSchema, {
                id: get.uuid(),
                name: 'Wake Up, You need to take kids to the school!',
                enabled: true,
                sound: getSound(),
                repeat: getRepeat(7),
                time: getTime([7, 8]),
                snoozeOptions: {
                    $typeName: 'braiins.bmc.web.SnoozeOptionsWrapper',
                    kind: {
                        case: 'snooze' as const,
                        value: {
                            $typeName: 'braiins.bmc.web.SnoozeOptions',
                            duration: pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES,
                            limit: pb.SnoozeLimit.SNOOZE_LIMIT_3,
                        },
                    },
                },
            }),
            pb.create(pb.AlarmSchema, {
                id: get.uuid(),
                name: 'Lunch break',
                enabled: false,
                sound: undefined,
                repeat: getRepeat([2, 3]),
                time: getTime([11, 12]),
                snoozeOptions: {
                    $typeName: 'braiins.bmc.web.SnoozeOptionsWrapper',
                    kind: {
                        case: 'off' as const,
                        value: pb.create(pb.OffSchema),
                    },
                },
            }),
            pb.create(pb.AlarmSchema, {
                id: get.uuid(),
                name: 'Alarm',
                enabled: false,
                sound: undefined,
                repeat: getRepeat([4, 5]),
                time: getTime([15, 16]),
                snoozeOptions: {
                    $typeName: 'braiins.bmc.web.SnoozeOptionsWrapper',
                    kind: {
                        case: 'off' as const,
                        value: pb.create(pb.OffSchema),
                    },
                },
            }),
            pb.create(pb.AlarmSchema, {
                id: get.uuid(),
                name: 'Weekend lazy ass…',
                enabled: false,
                sound: undefined,
                repeat: [pb.Weekday.SATURDAY, pb.Weekday.SUNDAY],
                time: getTime([10, 11]),
                snoozeOptions: getSnooze(),
            }),
        ],
    },
} satisfies Meta<AlarmsTableProps>;

export function AlarmsTable(args: AlarmsTableProps) {
    return <Component {...args} />;
}
