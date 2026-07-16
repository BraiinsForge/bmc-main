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

import cn from 'clsx';
import css from './Layout.scss';

export interface LayoutProps {
    header: ReactNode;
    footer?: null | MaybeArray<ReactElement>;
    children: ReactNode;
    className?: string;
    style?: CSSProperties;
}
export function Layout(props: LayoutProps): ReactElement {
    const { header, children, footer, className, style } = props;

    return (
        <div className={cn(css.root, className)} style={style}>
            <header className={css.header} children={header} />
            <main className={css.main} children={children} />
            {footer == null || (Array.isArray(footer) && footer.length === 0) ? null : (
                <footer className={css.footer} children={footer} />
            )}
        </div>
    );
}
