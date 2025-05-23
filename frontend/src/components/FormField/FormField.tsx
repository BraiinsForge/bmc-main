import type { HTMLAttributes } from 'react';

import cn from 'clsx';
import css from './FormField.scss';

type Text = string | ReactElement;
export interface FormFieldProps extends HTMLAttributes<HTMLDivElement> {
    labelText?: null | Text;
    helperText?: null | Text;
    error?: null | Text;
}

export function FormField(props: FormFieldProps) {
    const { labelText, helperText, error, children, className, ...rest } = props;

    let below: ReactNode = null;
    if (error != null) {
        below = <div role="alert" dir="auto" className={cn('cds--form-requirement', css.error)} children={error} />;
    } else if (helperText != null) {
        below ||= <div dir="auto" className="cds--form__helper-text" children={helperText} />;
    }

    return (
        <div {...rest} className={cn(css.root, error != null && css.invalid, className)}>
            {labelText != null && <div className="cds--label" children={labelText} />}
            {children}
            {below}
        </div>
    );
}
