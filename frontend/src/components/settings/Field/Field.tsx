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

import { useRef, useCallback, type HTMLAttributes } from 'react';

import cn from 'clsx';
import css from './Field.scss';

export interface FieldProps extends Omit<HTMLAttributes<HTMLInputElement>, 'title'> {
    title: ReactNode;
    description?: ReactNode;
    disabled?: boolean;
    children: ReactNode;

    variant?: 'light' | 'dark';

    className?: string;
    style?: CSSProperties;
}

export function Field(props: FieldProps) {
    const { title, description, disabled, children, variant = 'dark', className, ...rest } = props;

    const widgetRef = useRef<null | HTMLDivElement>(null);
    const handleClick = useCallback(() => {
        widgetRef.current?.querySelector<HTMLElement>('input,select,button')?.focus();
    }, []);

    return (
        <div {...rest} className={cn(css.root, css[`variant-${variant}`], disabled && css.disabled, className)}>
            {/* biome-ignore lint/a11y/useKeyWithClickEvents: Just a click helper, irrelevant for keyboard navigation */}
            <div onClick={handleClick} className={css.title} children={title} />
            {/* biome-ignore lint/a11y/useKeyWithClickEvents: Just a click helper, irrelevant for keyboard navigation */}
            <div onClick={handleClick} className={css.description} children={description} />
            <div className={css.widget} children={children} ref={widgetRef} />
        </div>
    );
}
