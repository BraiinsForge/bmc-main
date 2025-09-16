import type { Meta, StoryObj } from '@storybook/react';
import { Placeholder as Component, type PlaceholderProps as Props } from './Placeholder';

export default {
    title: 'accounts/Placeholder',
    component: Component,
    render(args) {
        return (
            <div
                style={{ display: 'inline-flex', background: 'var(--cds-layer-01)' }}
                children={<Component {...args} />}
            />
        );
    },
} satisfies Meta<Props>;

export const Small: StoryObj<Props> = { args: { rowsCount: 3 } };
export const Big: StoryObj<Props> = { args: { rowsCount: 9 } };
