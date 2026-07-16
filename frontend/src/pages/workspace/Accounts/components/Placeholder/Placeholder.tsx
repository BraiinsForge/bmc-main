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

// Styles
import css from './Placeholder.scss';
import cn from 'clsx';

export interface PlaceholderProps extends HTMLAttributes<HTMLTableElement> {
    rowsCount?: number;
}

export function Placeholder(props: PlaceholderProps) {
    const { rowsCount = 3, className, ...rest } = props;
    const rowOpacityBase = 0.6;
    const rowOpacityStep = rowOpacityBase / rowsCount;

    return (
        <table {...rest} className={cn(css.table, className)}>
            <thead>
                <tr>
                    <th children={<Box width={25} height={12} />} />
                    <th children={<Box width={90} height={12} />} />
                    <th children={<Box width={75} height={12} />} />
                    <th children={<Box width={97} height={12} />} />
                    <th children={<Box width={46} height={12} />} />
                    <th children={<Box width={72} height={12} />} />
                </tr>
            </thead>
            <tbody
                children={Array.from({ length: rowsCount }).map((_, i) => (
                    <tr
                        key={i}
                        style={{
                            opacity: rowOpacityBase - i * rowOpacityStep,
                        }}
                    >
                        <td children={<Box width={40} height={32} />} />
                        <td children={<Box width={128} height={12} />} />
                        <td children={<Box width={128} height={12} />} />
                        <td children={<Box width={128} height={12} />} />
                        <td children={<Box width={68} height={12} />} />
                        <td>
                            <Box width={54} height={28} style={{ marginInlineEnd: 16 }} />
                            <Box width={40} height={28} style={{ marginInlineEnd: 16 }} />
                            <Box width={40} height={28} />
                        </td>
                    </tr>
                ))}
            />
        </table>
    );
}

interface BoxProps {
    width: CSSProperties['width'];
    height: CSSProperties['height'];
    style?: Omit<CSSProperties, 'width' | 'height'>;
}
function Box({ width, height, style }: BoxProps) {
    return <div className={css.box} style={{ width, height, ...style }} />;
}
