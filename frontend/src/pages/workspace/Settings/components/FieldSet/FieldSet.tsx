import css from './FieldSet.scss';

export interface FieldSetProps {
    title: ReactNode;
    children: ReactNode;
}

export function FieldSet(props: FieldSetProps) {
    const { title, children } = props;

    return (
        <fieldset className={css.root}>
            <h1 className={css.title} children={title} />
            <div className={css.body} children={children} />
        </fieldset>
    );
}
