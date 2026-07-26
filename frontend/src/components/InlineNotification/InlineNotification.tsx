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

import css from './InlineNotification.scss';
import THEME from '@/styles/theme';
import cn from 'clsx';

import { ActionableNotification as BaseComponent, type ActionableNotificationProps as BaseProps } from '@carbon/react';

export type InlineNotificationProps = Omit<BaseProps, 'title' | 'actions'> & {
    stretch?: boolean;
    title?: ReactNode;
    children?: null | ReactNode;
    theme?: keyof typeof THEME;
    action?: {
        label: string;
        onClick(): void;
    };
};

export function InlineNotification(props: InlineNotificationProps) {
    const { stretch, className, kind, theme, action, children, title, ...rest } = props;

    if (!children && !title) return null;

    let kindClass: string = '';
    switch (kind) {
        case 'info':
            kindClass = css.info;
            break;

        case 'warning':
            kindClass = css.warning;
            break;

        case 'error':
            kindClass = css.error;
            break;

        case 'success':
            kindClass = css.success;
            break;

        default:
            kindClass = css.info;
    }

    const resProps = {
        ...rest,
        children: children != null ? <div className={css.children} children={children} /> : null,
        kind,
        lowContrast: false,
        title: title || '',
        className: cn(css.root, stretch && css.stretch, kindClass, theme && THEME[theme], className),
    } as BaseProps;

    if (action) {
        Object.assign(resProps, {
            actionButtonLabel: action.label,
            onActionButtonClick: action.onClick,
        });
    }

    return <BaseComponent {...resProps} />;
}
