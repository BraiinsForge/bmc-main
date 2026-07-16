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

import { Checkbox as Ch, type CheckboxProps as ChP } from '@carbon/react';

import cn from 'clsx';
import css from './Checkbox.scss';

type PropsBase = Omit<ChP, 'labelText' | 'onClick'>;
export interface CheckboxProps extends PropsBase {
    // Makes the checkbox behave like it's just a visual prop
    noop?: boolean;

    label?: null | ReactNode;
    labelWrap?: boolean;
    labelClassName?: string;

    description?: ReactNode;
    descriptionClassName?: string;

    invalid?: boolean;
    invalidText?: ReactNode;
}

export function Checkbox(props: CheckboxProps) {
    const {
        noop,
        label,
        labelWrap,
        labelClassName,
        description,
        descriptionClassName,
        invalid,
        invalidText,
        className,
        ...rest
    } = props;

    const labelText: ReactNode[] = [];

    if (label != null && label !== '') {
        labelText.push(
            <div key="label" className={cn(css.label, labelClassName, labelWrap && css.labelWrap)} children={label} />,
        );
    }

    if (description || (invalid && invalidText)) {
        labelText.push(
            <div
                key="description"
                className={cn(css.desc, descriptionClassName)}
                children={invalidText || description}
            />,
        );
    }

    return (
        <Ch
            {...rest}
            className={cn(css.root, invalid && css.invalid, noop && css.noop, className)}
            tabIndex={noop ? -1 : 0}
            labelText={labelText.length ? <div className={css.wrap} children={labelText} /> : ''}
        />
    );
}
