import css from './WidgetName.scss';

export interface WidgetNameProps {
    name: string;
    // Optional secondary label; omitted when absent.
    subname?: Maybe<string>;
}

// Name with an optional grayed subname that wraps to its own line, never mid-name.
export function WidgetName(props: WidgetNameProps) {
    const { name, subname } = props;
    return (
        <span className={css.root}>
            <span className={css.name} children={name} />
            {subname ? <span className={css.subname} children={subname} /> : null}
        </span>
    );
}
