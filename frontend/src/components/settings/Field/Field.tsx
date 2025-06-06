import { useRef, useCallback } from 'react';

import cn from 'clsx';
import css from './Field.scss';

export interface FieldProps {
    title: ReactNode;
    description?: ReactNode;
    disabled?: boolean;
    children: ReactNode;

    variant?: 'light' | 'dark';

    className?: string;
    style?: CSSProperties;
}

export function Field(props: FieldProps) {
    const { title, description, disabled, children, variant = 'dark', className, style } = props;

    const widgetRef = useRef<null | HTMLDivElement>(null);
    const handleClick = useCallback(() => {
        widgetRef.current?.querySelector<HTMLElement>('input,select,button')?.focus();
    }, []);

    return (
        <div style={style} className={cn(css.root, css[`variant-${variant}`], disabled && css.disabled, className)}>
            {/* biome-ignore lint/a11y/useKeyWithClickEvents: Just a click helper, irrelevant for keyboard navigation */}
            <div onClick={handleClick} className={css.title} children={title} />
            {/* biome-ignore lint/a11y/useKeyWithClickEvents: Just a click helper, irrelevant for keyboard navigation */}
            <div onClick={handleClick} className={css.description} children={description} />
            <div className={css.widget} children={children} ref={widgetRef} />
        </div>
    );
}
