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

import type { CarbonIconType } from '@/lib/react';

import css from './Empty.scss';
import cn from 'clsx';

export type EmptyProps = {
    icon: CarbonIconType;
    iconSize?: number;
    standaloneIcon?: boolean;

    title?: ReactNode;
    message?: ReactNode;
    controls?: ReactNode;

    fullWidth?: boolean;
    transparent?: boolean;
    kind?: 'info' | 'error';

    // Pass-through
    className?: string;
    style?: CSSProperties;
};

export function Empty(props: EmptyProps) {
    const {
        icon: Icon,
        iconSize,
        standaloneIcon,
        title,
        message,
        controls,
        kind,
        fullWidth,
        transparent,
        className,
        style,
    } = props;

    return (
        <div
            className={cn(
                css.root,
                fullWidth && css.fullWidth,
                transparent && css.transparent,
                standaloneIcon && css.standaloneIcon,
                css[kind || 'info'],
                className,
            )}
            style={style}
        >
            <div className={css.header}>
                <Icon size={iconSize || 48} className={css.icon} />
                {title && <div className={css.title} children={title} />}
            </div>
            {message && <div className={css.message} children={message} />}
            {controls && <div className={css.controls} children={controls} />}
        </div>
    );
}
