import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import type { iField } from '@/lib/form';

import * as pb from '@/proto';
import * as gen from '@/mocks';

import { SectionDisplay as Component, type SectionDisplayProps } from './SectionDisplay';

function getArg<T>(name: string, value: T): iField<T> {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'settings/components/SectionDisplay',
    component: Component,
    args: {
        brightness: getArg('brightnessDay', 75),
        nightBrightness: getArg('nightBrightness', 25),
        nightEnabled: getArg('nightEnabled', true),
        nightUseLocation: getArg('nightUseLocation', true),
        nightLocation: getArg('nightLocation', 'Berlin'),
        onLocationDetect: action('onLocationDetect'),
        nightNotify: getArg('nightNotify', true),
        nightInterval: getArg('nightInterval', pb.create(pb.TimeIntervalSchema, { from: '01:23', to: '12:34' })),
    } satisfies SectionDisplayProps,
} satisfies Meta<SectionDisplayProps>;

export function SectionDisplay(args: SectionDisplayProps) {
    return <Component {...args} />;
}
