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

import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import * as gen from '@/mocks';

import * as pb from '@/proto';
import type { iField } from '@/lib/form';
import { Setup as Component, type SetupProps } from './Setup';

function getField<T>(name: string, value: T): iField<T> {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'init/Setup',
    component: Component,
    args: {
        async onSubmit(...args) {
            action('onSubmit')(...args);
            return true;
        },

        timezone: {
            ...getField<pb.Timezone>('timezone', gen.randomItem(gen.timezones)),
            items: gen.timezones,
        },
        timeFormat: getField('timeFormat', pb.TimeFormat.TIME_FORMAT_24_HOUR),
        dateFormat: getField('dateFormat', pb.DateFormat.D_M_YYYY_SLASH),
        numberFormat: getField('numberFormat', pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL),
        temperatureUnits: getField('temperatureUnits', pb.TemperatureUnit.CELSIUS),
        unitSystem: getField('unitSystem', pb.UnitSystem.METRIC),

        password1: getField('password1', ''),
        password2: getField('password2', ''),

        // dataCollection: getField('password2', true),
    } satisfies SetupProps,
} satisfies Meta<SetupProps>;

export function Setup(args: SetupProps) {
    return <Component {...args} />;
}
