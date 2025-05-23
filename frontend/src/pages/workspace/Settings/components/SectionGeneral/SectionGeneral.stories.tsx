import { action } from '@storybook/addon-actions';
import type { Meta } from '@storybook/react';
import * as gen from '@/mocks';

import {
    SectionGeneral as Component,
    type SectionGeneralProps,
    Temperature,
    TimeFormat,
    WeekDay,
} from './SectionGeneral';

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
        timeFormat: getArg('timeFormat', TimeFormat.twelve),
        secondsInStatusbar: getArg('secondsInStatusbar', true),
        timezone: {
            ...getArg('timezone', gen.randomItem(gen.timezones)),
            items: gen.timezones,
        },
        dateFormat: getArg('dateFormat', 'DMY_SLASH'),
        firstWeekDay: getArg('dateFormat', WeekDay.Monday),
        temperature: getArg('dateFormat', Temperature.C),
        numberFormat: getArg('dateFormat', 'spaceAndComma'),
        onFactoryReset: action('onFactoryReset'),
    } satisfies SectionGeneralProps,
} satisfies Meta<SectionGeneralProps>;

export function SectionGeneral(args: SectionGeneralProps) {
    return <Component {...args} />;
}
