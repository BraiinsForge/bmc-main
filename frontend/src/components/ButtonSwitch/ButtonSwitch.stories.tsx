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
