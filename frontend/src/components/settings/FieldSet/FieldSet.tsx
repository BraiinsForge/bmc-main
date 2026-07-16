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

import css from './FieldSet.scss';

export interface FieldSetProps {
    title: null | ReactNode;
    description?: ReactNode;
    children: ReactNode;
}

export function FieldSet(props: FieldSetProps) {
    const { title, description, children } = props;

    return (
        <fieldset className={css.root}>
            {title != null ? <h1 className={css.title} children={title} /> : null}
            {description != null ? <p className={css.description} children={description} /> : null}
            <div className={css.body} children={children} />
        </fieldset>
    );
}
