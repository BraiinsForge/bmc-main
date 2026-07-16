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

import { useIntl } from 'react-intl';
import { Close } from '@carbon/react/icons';

import css from './Progressbar.scss';
import cn from 'clsx';

export interface ProgressbarSegment {
    value: number;
    color?: string;
    animate?: boolean;
}
export interface ProgressbarProps {
    values: ProgressbarSegment[];
    valueUpperBound?: 1 | 100;

    label?: ReactNode;
    labelPosition?: 'top-right' | 'top-left' | 'bottom-left';

    shadow?: boolean;
    height?: CSSProperties['height'];
    bgColor?: string;

    onCancel?(): void;
    cancelTitle?: string;

    className?: string;
    style?: CSSProperties;
}

export const Progressbar = (props: ProgressbarProps) => {
    const {
        values,
        valueUpperBound,
        label,
        labelPosition,
        shadow,
        height,
        bgColor,
        onCancel,
        cancelTitle,
        className,
        style,
    } = props;
    const { formatMessage } = useIntl();

    return (
        <div
            className={cn(css.outer, labelPosition && css[labelPosition], shadow && css.shadow, className)}
            style={style}
        >
            <div className={css.content}>
                <div
                    className={css.values}
                    style={{ backgroundColor: bgColor, height }}
                    children={values.map((x, i) => (
                        <div
                            key={i}
                            className={cn(css.line, x.animate && css.animated)}
                            style={{ width: `${getWidth(x.value, valueUpperBound)}%`, backgroundColor: x.color }}
                        />
                    ))}
                />

                {typeof onCancel === 'function' ? (
                    <button
                        type="button"
                        onClick={onCancel}
                        className={css.cancelButton}
                        title={cancelTitle || formatMessage({ defaultMessage: 'Cancel' })}
                        children={<Close size={16} />}
                    />
                ) : null}
            </div>
            {label != null ? <div className={css.label} children={label} /> : null}
        </div>
    );
};

function getWidth(value: number, valueUpperBound: ProgressbarProps['valueUpperBound']): number {
    let width = 0;
    if (typeof value === 'number' && Number.isFinite(value)) {
        const v = !valueUpperBound || valueUpperBound === 1 ? value * 100 : value;
        width = Math.min(Math.max(v, 0), 100);
    }
    return width;
}
