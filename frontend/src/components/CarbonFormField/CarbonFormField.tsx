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

import cn from 'clsx';
import css from './CarbonFormField.scss';

type Text = string | ReactElement;
export interface CarbonFormFieldProps extends HTMLAttributes<HTMLDivElement> {
    labelText?: null | Text;
    helperText?: null | Text;
    error?: null | Text;
}

export function CarbonFormField(props: CarbonFormFieldProps) {
    const { labelText, helperText, error, children, className, ...rest } = props;

    let below: ReactNode = null;
    if (error != null) {
        below = <div role="alert" dir="auto" className={cn('cds--form-requirement', css.error)} children={error} />;
    } else if (helperText != null) {
        below ||= <div dir="auto" className="cds--form__helper-text" children={helperText} />;
    }

    return (
        <div {...rest} className={cn(css.root, error != null && css.invalid, className)}>
            {labelText != null && <div className="cds--label" children={labelText} />}
            {children}
            {below}
        </div>
    );
}
