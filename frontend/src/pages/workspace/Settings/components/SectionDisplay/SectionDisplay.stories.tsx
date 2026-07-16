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
