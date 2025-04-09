import cn from 'clsx';
import css from './ButtonGroup.scss';

export type ButtonGroupProps = {
    children: ReactNode;
    spaced?: boolean;
    vertical?: boolean;

    className?: string;
    style?: CSSProperties;
};

export function ButtonGroup({ className, spaced, vertical, ...props }: ButtonGroupProps) {
    return (
        <div
            {...props}
            role="group"
            className={cn('cds--btn-set', css.root, spaced && css.spaced, vertical && css.vertical, className)}
        />
    );
}
