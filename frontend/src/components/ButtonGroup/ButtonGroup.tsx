import cn from 'clsx';
import css from './ButtonGroup.scss';

export interface ButtonGroupProps {
    children: ReactNode;
    spaced?: boolean;
    vertical?: boolean;

    className?: string;
    style?: CSSProperties;
}
export function ButtonGroup(props: ButtonGroupProps) {
    const { className, spaced, vertical, ...rest } = props;
    return (
        <div
            {...rest}
            role="group"
            className={cn('cds--btn-set', css.root, spaced && css.spaced, vertical && css.vertical, className)}
        />
    );
}
