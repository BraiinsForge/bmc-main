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

import { Fragment, type HTMLAttributes } from 'react';
import { format } from 'd3-format';

// n sigfig, SI preffixed, trim trailing zeroes
const fmt = format('.4~s');

export interface SiUnitProps extends HTMLAttributes<HTMLSpanElement> {
    value: Maybe<number>;

    prefix?: string;
    unitPrefix?: string;
    unit: string;

    placeholder?: ReactNode;
}

export default function SiUnit(props: SiUnitProps) {
    const { value, prefix, unitPrefix, unit, placeholder = '---', ...rest } = props;
    let content = <span data-role="value" children={placeholder} />;

    if (value != null && Number.isFinite(value)) {
        const [_, v, u] = fmt(value).match(/([\d.]+)(.*)/i) ?? [null, String(placeholder), ''];

        content = (
            <Fragment>
                {prefix && (
                    <Fragment>
                        <span data-role="unit" children={prefix} />
                        &nbsp;
                    </Fragment>
                )}
                <span data-role="value" children={v.trim()} />
                &nbsp;
                <span data-role="unit" children={`${unitPrefix ?? ''}${u.trim()}${unit}`} />
            </Fragment>
        );
    }

    return <span dir="ltr" {...rest} children={content} />;
}
