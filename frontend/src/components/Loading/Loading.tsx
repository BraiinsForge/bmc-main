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

import type { HTMLAttributes } from 'react';
import { isPlainObject } from 'es-toolkit';

import cn from 'clsx';
import css from './Loading.scss';

export type LoadingProps = {
    size?: string | number;
    active?: boolean;
    wrapper?: string | HTMLAttributes<HTMLDivElement>; // ClassName
    cover?: boolean;

    className?: string;
    style?: CSSProperties;
};

export function Loading({ size, active, wrapper, cover, ...rest }: LoadingProps) {
    if (active !== true) return null;

    const className = cn(css.root, rest.className);
    const style = { ...rest.style };
    if (size) style.width = style.height = size;

    const content = (
        <div {...rest} className={className} style={style}>
            <svg className={css.spinner} viewBox="25 25 50 50">
                <circle className={css.bg} cx="50" cy="50" r="20" fill="none" strokeMiterlimit="10" />
                <circle className={css.fg} cx="50" cy="50" r="20" fill="none" strokeMiterlimit="10" />
            </svg>
            <svg viewBox="0 0 34 32" className={css.ii}>
                <path d="M7.10151.00481167 0 0v5.54545L6.73872 27.4794V32h7.10148v-5.5454L7.10151 4.51816V.00481167Zm20.15979 0L20.1573 0v5.54545L26.896 27.4794V32H34v-5.5454L27.2613 4.51816V.00481167Z" />
            </svg>
        </div>
    );

    if (wrapper || cover) {
        const wrapperProps: HTMLAttributes<HTMLDivElement> = {};
        if (cover) wrapperProps.className = css.cover;
        else if (typeof wrapper === 'string') wrapperProps.className = wrapper;
        else if (isPlainObject(wrapper)) Object.assign(wrapperProps, wrapper);
        return <div {...wrapperProps} children={content} />;
    }
    return content;
}
