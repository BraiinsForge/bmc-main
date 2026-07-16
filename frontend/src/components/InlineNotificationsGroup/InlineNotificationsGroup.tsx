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

import type { ReactElement } from 'react';

import cn from 'clsx';
import THEME from '@/styles/theme';
import css from './InlineNotificationsGroup.scss';

import { InlineNotification, type InlineNotificationProps } from '../InlineNotification';

export type ErrorItem = undefined | null | false | string | InlineNotificationProps;
export type InlineNotificationsGroupProps = {
    items?: Maybe<Array<ErrorItem>>;
    renderedItems?: Array<ReactElement<any, typeof InlineNotification>>;

    stretch?: boolean;
    theme?: keyof typeof THEME;
    kind?: InlineNotificationProps['kind'];

    className?: string;
    style?: CSSProperties;
};

export function InlineNotificationsGroup(props: InlineNotificationsGroupProps) {
    const { items, renderedItems, className, kind, theme, stretch, ...rest } = props;
    const children: ReactNode[] = [];

    items?.forEach((d, i) => {
        if (d == null || d === false) return;

        const itemProps: InlineNotificationProps = {
            stretch,
            kind: kind || 'info',
            hideCloseButton: true,
        };
        if (typeof d === 'string') itemProps.children = d;
        else Object.assign(itemProps, d);

        children.push(<InlineNotification key={i} {...itemProps} />);
    });
    if (renderedItems?.length) children.push(...renderedItems);

    if (!children.length) return null;

    return (
        <div
            {...rest}
            role="alert"
            className={cn(css.root, theme && THEME[theme], stretch && css.stretch, className)}
            children={children}
        />
    );
}
