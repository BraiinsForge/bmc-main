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

import { format } from 'd3-format';

export interface PercentageProps {
    value?: null | number; // 0..1 | 0..100

    /**
     * Log10 multiplier for the value defaulting to 0.
     *
     * @example
     * `<Luck value={500} base={-2} precision={2} />` == `500 * 1e-2` == `50%`
     */
    base?: number;

    /**
     * Value less or equal to this will be treated
     * as nil and placeholder will be shown
     */
    lowerValueBound?: number;
    /**
     * Decides the domain in which the input value is treated…
     * that is float "from 0 to 1" or "from 1 to 100"
     */
    upperValueBound?: 1 | 100;

    round?: boolean | number;
    trim?: boolean; // Remove trailing zeros

    placeholder?: string;

    // Visuals
    className?: string;
    style?: CSSProperties;
}
export function Percentage(props: PercentageProps) {
    const {
        value,
        base,
        lowerValueBound = 0.0001,
        upperValueBound = 1,
        round,
        trim,
        placeholder = '< 0.01',
        ...rest
        //
    } = props;

    let formated = placeholder;
    if (value != null && Number.isFinite(value)) {
        let normalized: number = value;
        if (base && Number.isFinite(base)) normalized = value * 10 ** base;
        if (upperValueBound === 1) normalized *= 100;

        if (normalized > lowerValueBound) {
            const frac = typeof round === 'number' ? round : round ? 0 : 2;
            const fmt = trim
                ? // Fixed decimals, no trailing zeroes
                  format(`.${frac}~f`)
                : // Fixed number of decimals
                  format(`.${frac}f`);
            formated = fmt(normalized);
        }
    }

    return (
        <span dir="ltr" {...rest}>
            <data data-role="value" children={formated} />
            {/*
             English style guides prescribe writing the percent sign
             following the number without any space between (e.g. 50%).
            */}
            <span role="presentation" data-role="unit" children="%" />
        </span>
    );
}
