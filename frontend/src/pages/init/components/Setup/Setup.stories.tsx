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

        password1: getField('password1', ''),
        password2: getField('password2', ''),

        dataCollection: getField('password2', true),
    } satisfies SetupProps,
} satisfies Meta<SetupProps>;

export function Setup(args: SetupProps) {
    return <Component {...args} />;
}
