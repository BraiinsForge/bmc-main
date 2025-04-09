import { Fragment, type HTMLAttributes } from 'react';
import { format } from 'd3-format';

// n sigfig, SI preffixed, trim trailing zeroes
const fmt = format('.4~s');

export interface SiUnitProps extends HTMLAttributes<HTMLSpanElement> {
    value: Maybe<number>;

    prefix?: string;
    unitPrefix?: string;
    unit: string;

    placeholder?: ReactNode;
}

export default function SiUnit(props: SiUnitProps) {
    const { value, prefix, unitPrefix, unit, placeholder = '---', ...rest } = props;
    let content = <span data-role="value" children={placeholder} />;

    if (value != null && Number.isFinite(value)) {
        const [_, v, u] = fmt(value).match(/([\d.]+)(.*)/i) ?? [null, String(placeholder), ''];

        content = (
            <Fragment>
                {prefix && (
                    <Fragment>
                        <span data-role="unit" children={prefix} />
                        &nbsp;
                    </Fragment>
                )}
                <span data-role="value" children={v.trim()} />
                &nbsp;
                <span data-role="unit" children={`${unitPrefix ?? ''}${u.trim()}${unit}`} />
            </Fragment>
        );
    }

    return <span dir="ltr" {...rest} children={content} />;
}
