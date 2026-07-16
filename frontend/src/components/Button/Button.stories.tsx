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

import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import { Home as IconHome } from '@carbon/react/icons';

import { uuid } from '@/mocks';
import { Button as B, type ButtonProps } from './Button';

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
    const getBlock = (comment: string, args: Omit<ButtonProps, 'id'>) => {
        return (
            <div key={comment} style={{ display: 'flex', flexDirection: 'column', gap: '1rem', padding: '0.5rem' }}>
                <h1 children={comment} />
                {kinds.map(kind => {
                    return <B id={uuid()} key={kind} kind={kind} title="Button Title" {...args} />;
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
