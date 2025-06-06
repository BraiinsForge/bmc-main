import css from './FieldSet.scss';

export interface FieldSetProps {
    title: null | ReactNode;
    description?: ReactNode;
    children: ReactNode;
}

export function FieldSet(props: FieldSetProps) {
    const { title, description, children } = props;

    return (
        <fieldset className={css.root}>
            {title != null ? <h1 className={css.title} children={title} /> : null}
            {description != null ? <p className={css.description} children={description} /> : null}
            <div className={css.body} children={children} />
        </fieldset>
    );
}
