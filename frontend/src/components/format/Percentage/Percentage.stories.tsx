import type { Meta } from '@storybook/react';
import { Percentage as Component, type PercentageProps } from './Percentage';

export default {
    title: 'components/format/Percentage',
    component: Component,
    args: {
        value: 2,
        placeholder: '---',
    } satisfies PercentageProps,
} satisfies Meta<PercentageProps>;

export function Percentage(args: PercentageProps) {
    return <Component className="ui-box" {...args} />;
}
