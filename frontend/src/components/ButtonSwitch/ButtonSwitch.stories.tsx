// Copyright (C) 2025  Braiins Systems s.r.o.
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
import { Debug } from '@carbon/react/icons';
import { range } from 'es-toolkit';

import { arrayOf } from '@/mocks';
import { ButtonSwitch as Component, type ButtonSwitchProps } from './ButtonSwitch';

type Props = ButtonSwitchProps<number, string>;
const hide = Object.freeze({ table: { disable: true } });
const itemsCount = 6;

export default {
    title: 'components/ButtonSwitch',
    component: Component,
    args: {
        selectedOption: 0,
        options: arrayOf(itemsCount, i => ({
            id: i,
            text: `item #${i + 1}`,
            icon: Debug,
        })),

        onChange: action('onChange'),
        onClick: action('onClick'),

        disabled: false,
        invalid: false,
    } satisfies Props,
    argTypes: {
        selectedOption: {
            options: range(0, itemsCount),
            control: { type: 'inline-radio' },
        },
        onChange: hide,
        onClick: hide,
    },
} satisfies Meta;

export const ButtonSwitch = (args: ButtonSwitchProps) => <Component {...args} style={{ margin: 40 }} />;
ButtonSwitch.storyName = 'ButtonSwitch';
