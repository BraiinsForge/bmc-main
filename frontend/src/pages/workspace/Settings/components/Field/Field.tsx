import css from './Field.scss';

export interface FieldProps {
    title: ReactNode;
    description?: ReactNode;
    children: ReactNode;
}

export function Field(props: FieldProps) {
    const { title, description, children } = props;

    return (
        <div className={css.root}>
            <div className={css.title} children={title} />
            <div className={css.description} children={description} />
            <div className={css.widget} children={children} />
        </div>
    );
}
