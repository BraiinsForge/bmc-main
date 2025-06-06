import { action } from 'storybook/actions';

import { Button as B, type ButtonProps } from './Button';
import type { Meta } from '@storybook/react';
import { Home as IconHome } from '@carbon/react/icons';

const kinds: Array<ButtonProps['kind']> = ['primary', 'secondary', 'tertiary', 'ghost'];

export default {
    title: 'components/Button',
    component: B,
    args: {
        loading: false,
        kind: kinds[0],
    },
    argTypes: {
        loading: {
            control: 'boolean',
        },
        kind: {
            control: 'select',
            options: kinds,
        },
    },
} satisfies Meta<typeof B>;

export function Button() {
    // Common args for easy and readable reuse in different cases
    const href = 'https://example.org';
    const target = '_blank';
    const children = 'Button';
    const icon = IconHome;
    const onClick = action('onClick');

    // Bound renderer to avoid repetition & unly supply changing props
    const getBlock = (comment: string, args: ButtonProps) => {
        return (
            <div key={comment} style={{ display: 'flex', flexDirection: 'column', gap: '1rem', padding: '0.5rem' }}>
                <h1 children={comment} />
                {kinds.map(kind => {
                    return <B key={kind} kind={kind} title="Button Title" {...args} />;
                })}
            </div>
        );
    };

    return [
        getBlock('Anchor', { href, target }),
        getBlock('Anchor with icon', { href, target, icon }),
        getBlock('Text only', { children, onClick }),
        getBlock('Icon only', { icon, onClick }),
        getBlock('Icon and text', { icon, children, onClick }),
    ];
}
