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
        brightness: getArg(
            'brightnessDay',
            pb.create(pb.BrightnessInfoSchema, {
                value: 75,
                min: 0,
                max: 100,
                step: 1,
            }),
        ),
        nightBrightness: getArg(
            'nightBrightness',
            pb.create(pb.BrightnessInfoSchema, {
                value: 25,
                min: 0,
                max: 100,
                step: 1,
            }),
        ),
        nightEnabled: getArg('nightEnabled', true),
        nightUseLocation: getArg('nightUseLocation', true),
        nightLocation: getArg('nightLocation', 'Berlin'),
        onLocationDetect: action('onLocationDetect'),
        nightNotify: getArg('nightNotify', true),
        nightScreenOffTimeout: getArg('nightScreenOffTimeout', 0),
        nightInterval: {
            ...getArg('nightInterval', pb.create(pb.TimeIntervalSchema, { from: '01:23', to: '12:34' })),
            hasChanged: true,
            onConfirm: action('nightInterval.onConfirm'),
        },
    } satisfies SectionDisplayProps,
} satisfies Meta<SectionDisplayProps>;

export function SectionDisplay(args: SectionDisplayProps) {
    return <Component {...args} />;
}
