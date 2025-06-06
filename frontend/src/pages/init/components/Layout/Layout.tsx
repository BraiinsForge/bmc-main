import cn from 'clsx';
import css from './Layout.scss';

export interface LayoutProps {
    header: ReactNode;
    footer?: null | MaybeArray<ReactElement>;
    children: ReactNode;
    className?: string;
    style?: CSSProperties;
}
export function Layout(props: LayoutProps): ReactElement {
    const { header, children, footer, className, style } = props;

    return (
        <div className={cn(css.root, className)} style={style}>
            <header className={css.header} children={header} />
            <main className={css.main} children={children} />
            {footer == null || (Array.isArray(footer) && footer.length === 0) ? null : (
                <footer className={css.footer} children={footer} />
            )}
        </div>
    );
}
