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
