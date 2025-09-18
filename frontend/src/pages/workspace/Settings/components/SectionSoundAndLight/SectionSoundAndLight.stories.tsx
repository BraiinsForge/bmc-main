import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import type { iField } from '@/lib/form';

import * as pb from '@/proto';
import * as gen from '@/mocks';

import { SectionSoundAndLight as Component, type SectionSoundAndLightProps } from './SectionSoundAndLight';

function getArg<T>(name: string, value: T): iField<T> {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'settings/components/SectionSoundAndLight',
    component: Component,
    args: {
        soundVolume: getArg('brightnessDay', pb.create(pb.SoundVolumeSchema, { value: 22, min: 0, max: 100, step: 4 })),
        soundVolumeNight: getArg(
            'nightBrightness',
            pb.create(pb.SoundVolumeSchema, { value: 33, min: 0, max: 100, step: 4 }),
        ),
        // alarmAndNotifyVolume: getArg('nightEnabled', 44),
        ledNotifyEnabled: getArg('nightNotify', true),
    } satisfies SectionSoundAndLightProps,
} satisfies Meta<SectionSoundAndLightProps>;

export function SectionSoundAndLight(args: SectionSoundAndLightProps) {
    return <Component {...args} />;
}
