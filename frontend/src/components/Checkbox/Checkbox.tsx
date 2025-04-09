import { Checkbox as Ch, type CheckboxProps as ChP } from '@carbon/react';

import cn from 'clsx';
import css from './Checkbox.scss';

type PropsBase = Omit<ChP, 'labelText' | 'onClick'>;
export interface CheckboxProps extends PropsBase {
    // Makes the checkbox behave like it's just a visual prop
    noop?: boolean;

    label?: null | ReactNode;
    labelWrap?: boolean;
    labelClassName?: string;

    description?: ReactNode;
    descriptionClassName?: string;

    invalid?: boolean;
    invalidText?: ReactNode;
}

export function Checkbox(props: CheckboxProps) {
    const {
        noop,
        label,
        labelWrap,
        labelClassName,
        description,
        descriptionClassName,
        invalid,
        invalidText,
        className,
        ...rest
    } = props;

    const labelText: ReactNode[] = [];

    if (label != null && label !== '') {
        labelText.push(
            <div key="label" className={cn(css.label, labelClassName, labelWrap && css.labelWrap)} children={label} />,
        );
    }

    if (description || (invalid && invalidText)) {
        labelText.push(
            <div
                key="description"
                className={cn(css.desc, descriptionClassName)}
                children={invalidText || description}
            />,
        );
    }

    return (
        <Ch
            {...rest}
            className={cn(css.root, invalid && css.invalid, noop && css.noop, className)}
            tabIndex={noop ? -1 : 0}
            labelText={labelText.length ? <div className={css.wrap} children={labelText} /> : ''}
        />
    );
}
