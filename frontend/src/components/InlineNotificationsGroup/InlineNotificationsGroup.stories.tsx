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

import type { ArgTypes } from '@storybook/react';
import lay from '@/styles/layout.scss';
import THEME from '@/styles/theme';
import cn from 'clsx';

import { InlineNotificationsGroup as Component } from './InlineNotificationsGroup';
import type { InlineNotificationProps } from '../InlineNotification';

const style = {
    cell: { padding: '0 80px 64px' },
    h1: {
        margin: '4px 0',
        padding: '0.5rem 1rem',
        fontSize: '16px',
        lineHeight: '16px',
        textAlign: 'center',
        background: '#fff',
        color: '#000',
        display: 'inline-block',
    },
} as const;
type Args = InlineNotificationProps & {
    $count: number;
};

export default {
    title: 'components/InlineNotificationsGroup',
    component: Component,
};

export const InlineNotificationsGroup = ({ $count, ...args }: Args) => {
    const items = Array.from<InlineNotificationProps>({ length: $count }).fill(args);

    return (
        <>
            <h1 style={{ ...style.h1 }} children={THEME.light} />
            <br />
            <section className={THEME.light}>
                <div className={cn(lay.vertical, 'dark')} style={{ padding: 32, backgroundColor: '#161616' }}>
                    <Component items={items} />
                </div>
                <div className={cn(lay.vertical)} style={{ padding: 32, backgroundColor: '#fff' }}>
                    <Component items={items} />
                </div>
            </section>

            <h1 style={{ ...style.h1 }} children={THEME.dark} />
            <br />
            <section className={THEME.dark}>
                <div className={cn(lay.vertical, 'dark')} style={{ padding: 32, backgroundColor: '#161616' }}>
                    <Component items={items} />
                </div>
                <div className={cn(lay.vertical)} style={{ padding: 32, backgroundColor: '#fff' }}>
                    <Component items={items} />
                </div>
            </section>
        </>
    );
};

InlineNotificationsGroup.storyName = 'InlineNotificationsGroup';
InlineNotificationsGroup.args = {
    kind: 'info',
    title: 'Title',
    children: 'Notification body text.',
    hideCloseButton: true,
    lowContrast: false,
    $count: 3,
} as Partial<Args>;
InlineNotificationsGroup.argTypes = {
    $count: {
        type: 'number',
        name: 'Number of items',
        control: { type: 'range', min: 1, max: 5, step: 1 },
    },
} as ArgTypes<Args>;
