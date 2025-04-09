export interface GenericProps {
    prefix?: string;
    value: Maybe<ReactNode>;

    unit: Maybe<ReactNode>;
    placeholder?: StrNum;

    style?: CSSProperties;
    className?: string;
}

export function Generic(props: GenericProps) {
    const { prefix, value, unit, placeholder = '---', ...rest } = props;
    let content = <span data-role="value" children={placeholder} />;

    if (value != null) {
        content = (
            <>
                {prefix && (
                    <>
                        <span data-role="unit" children={prefix} />
                        &nbsp;
                    </>
                )}
                <span data-role="value" children={value} />
                &nbsp;
                <span data-role="unit" children={unit} />
            </>
        );
    }

    return <span dir="ltr" {...rest} children={content} />;
}
