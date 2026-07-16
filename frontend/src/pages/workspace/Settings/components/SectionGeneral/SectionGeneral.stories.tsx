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

import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import * as gen from '@/mocks';

import { SectionGeneral as Component, type SectionGeneralProps } from './SectionGeneral';

function getArg<T>(name: string, value: T) {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'settings/components/SectionGeneral',
    component: Component,
    args: {
        timeFormat: getArg('timeFormat', pb.TimeFormat.TIME_FORMAT_12_HOUR),
        // secondsInStatusbar: getArg('secondsInStatusbar', true),
        timezone: {
            ...getArg('timezone', gen.randomItem(gen.timezones)),
            items: gen.timezones,
        },
        dateFormat: getArg('dateFormat', pb.DateFormat.DD_MM_YYYY_DASH),
        firstWeekDay: getArg('dateFormat', pb.Weekday.THURSDAY),
        temperatureUnits: getArg('temperatureUnits', pb.TemperatureUnit.CELSIUS),
        unitSystem: getArg('unitSystem', pb.UnitSystem.METRIC),
        numberFormat: getArg('dateFormat', pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL),

        onFactoryReset: action('onFactoryReset'),
        onSystemReboot: action('onSystemReboot'),
        onDownloadSupportArchive: action('onDownloadSupportArchive'),

        // usageData: getArg('usageData', true),
    } satisfies SectionGeneralProps,
} satisfies Meta<SectionGeneralProps>;

export function SectionGeneral(args: SectionGeneralProps) {
    return <Component {...args} />;
}
