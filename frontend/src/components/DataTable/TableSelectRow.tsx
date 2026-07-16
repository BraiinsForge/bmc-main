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
import { Checkbox } from '@/components/Checkbox';
import { RadioButton } from '@/components/RadioButton';

export interface TableSelectRowProps {
    // ids
    id: string;
    name: string;
    ariaLabel: string;

    // state
    checked: boolean;
    disabled?: boolean;
    radio: boolean;

    // callbacks
    onChange(checked: boolean): void;
    render?(children: ReactElement): ReactElement;

    // Visual
    className?: string;
    style?: CSSProperties;
}

export function TableSelectRow(props: TableSelectRowProps) {
    const {
        // ids
        id,
        name,
        ariaLabel,
        // state
        radio,
        checked,
        disabled,
        // callbacks
        onChange,
        render = x => x,
        // visual
        className,
        style,
    } = props;

    return (
        <td
            style={style}
            className={cn('cds--table-column-checkbox', radio && 'cds--table-column-radio', className)}
            children={render(
                radio ? (
                    <RadioButton
                        id={id}
                        name={name}
                        onChange={(_, __, e) => onChange((e.target satisfies HTMLInputElement).checked)}
                        checked={checked}
                        disabled={disabled}
                        label={ariaLabel}
                        hideLabel
                    />
                ) : (
                    <Checkbox
                        id={id}
                        name={name}
                        onChange={e => onChange(e.target.checked)}
                        checked={checked}
                        disabled={disabled}
                    />
                ),
            )}
        />
    );
}
