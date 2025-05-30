import cn from 'clsx';
import css from './Field.scss';

export interface FieldProps {
    title: ReactNode;
    description?: ReactNode;
    disabled?: boolean;
    children: ReactNode;

    className?: string;
    style?: CSSProperties;
}

export function Field(props: FieldProps) {
    const { title, description, disabled, children, className, style } = props;

    return (
        <div className={cn(css.root, disabled && css.disabled, className)} style={style}>
            <div className={css.title} children={title} />
            <div className={css.description} children={description} />
            <div className={css.widget} children={children} />
        </div>
    );
}
