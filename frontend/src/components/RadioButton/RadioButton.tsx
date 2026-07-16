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

import { RadioButton as RB, type RadioButtonProps as RBP } from '@carbon/react';

// Styles & testing
import css from './RadioButton.scss';
import cn from 'clsx';

type BaseProps = Omit<RBP, 'labelText'>;
export interface RadioButtonProps extends BaseProps {
    label?: NonNullable<ReactNode>;

    description?: ReactNode;
    descriptionClassName?: string;

    addon?: ReactNode;
    darker?: boolean;
    stretch?: boolean;
}

export function RadioButton(props: RadioButtonProps) {
    const {
        // Content
        label,
        description,
        descriptionClassName,
        addon,
        // Modifiers
        darker,
        stretch,
        // DOM
        className,
        style,
        ...rest
    } = props;

    let $label: ReactNode = label;
    if (label || description) {
        $label = (
            <span className={css.labelWrapper}>
                <span className={css.label} children={label} />
                {description && <span className={cn(css.description, descriptionClassName)} children={description} />}
            </span>
        );
    }

    if (addon) {
        $label = (
            <div className={css.addonWrapper}>
                {label}
                <div className={css.addon} children={addon} />
            </div>
        );
    }

    const cname = cn(css.root, darker && css.darker, stretch && css.stretch, className);
    return <RB {...rest} labelText={$label} className={cname} style={style} />;
}
