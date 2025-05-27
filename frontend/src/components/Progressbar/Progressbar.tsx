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
