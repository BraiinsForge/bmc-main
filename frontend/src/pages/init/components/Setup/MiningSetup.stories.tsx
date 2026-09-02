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
import { MiningSetup as Component, type MiningSetupProps } from './MiningSetup';

function getField<T>(name: string, value: T): iField<T> {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'init/MiningSetup',
    component: Component,
    args: {
        async onSubmit(...args) {
            action('onSubmit')(...args);
            return true;
        },

        poolUrl: getField('poolUrl', 'stratum+tcp://solo.stratum.braiins.com:3333'),
        poolUser: getField('poolUser', 'bc1qexampleworker'),
        poolPassword: getField('poolPassword', ''),

        hostname: getField('hostname', 'miner-01'),
        protocol: getField('protocol', 'static'),
        staticAddress: getField('staticAddress', '192.168.1.126'),
        staticNetmask: getField('staticNetmask', '255.255.255.0'),
        staticGateway: getField('staticGateway', '192.168.1.1'),
        staticDns: getField('staticDns', '1.1.1.1, 8.8.8.8'),

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
    } satisfies MiningSetupProps,
} satisfies Meta<MiningSetupProps>;

export function MiningSetup(args: MiningSetupProps) {
    return <Component {...args} />;
}
