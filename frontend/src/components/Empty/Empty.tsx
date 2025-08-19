import type { CarbonIconType } from '@/lib/react';

import css from './Empty.scss';
import cn from 'clsx';

export type EmptyProps = {
    icon: CarbonIconType;
    iconSize?: number;

    title?: ReactNode;
    message?: ReactNode;
    controls?: ReactNode;

    fullWidth?: boolean;
    transparent?: boolean;
    kind?: 'info' | 'error';

    // Pass-through
    className?: string;
    style?: CSSProperties;
};

export function Empty(props: EmptyProps) {
    const { icon: Icon, iconSize, title, message, controls, kind, fullWidth, transparent, className, style } = props;

    return (
        <div
            className={cn(
                css.root,
                fullWidth && css.fullWidth,
                transparent && css.transparent,
                css[kind || 'info'],
                className,
            )}
            style={style}
        >
            <div className={css.header}>
                <Icon size={iconSize || 48} className={css.icon} />
                {title && <div className={css.title} children={title} />}
            </div>
            {message && <div className={css.message} children={message} />}
            {controls && <div className={css.controls} children={controls} />}
        </div>
    );
}
