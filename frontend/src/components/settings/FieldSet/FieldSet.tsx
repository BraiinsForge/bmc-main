import css from './FieldSet.scss';

export interface FieldSetProps {
    title: null | ReactNode;
    children: ReactNode;
}

export function FieldSet(props: FieldSetProps) {
    const { title, children } = props;

    return (
        <fieldset className={css.root}>
            {title != null ? <h1 className={css.title} children={title} /> : null}
            <div className={css.body} children={children} />
        </fieldset>
    );
}
