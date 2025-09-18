import { useIntl } from 'react-intl';
import { getID } from '../../const';
import { Form, type iField } from '@/lib/form';

// Components
import { PasswordInput } from '@carbon/react';

// CSS
import css from './FormPasswordChange.scss';
import type { HTMLInputAutoCompleteAttribute } from 'react';

export interface FormPasswordChangeProps {
    passCurrent: null | iField<string>;
    passNew: null | iField<string>;
    passConfirm: null | iField<string>;
}
interface Props extends FormPasswordChangeProps {}

const $ = getID('password-change').get;

export function FormPasswordChange(props: Props) {
    const { formatMessage } = useIntl();

    const {
        // Fields
        passCurrent,
        passNew,
        passConfirm,
    } = props;

    return (
        <Form className={css.form}>
            {passCurrent == null ? null : (
                <Field
                    name="password-current"
                    label={formatMessage({ defaultMessage: 'Current Password' })}
                    autoComplete="current-password"
                    field={passCurrent}
                />
            )}

            {passNew == null ? null : (
                <Field
                    name="password-new"
                    autoComplete="new-password"
                    label={formatMessage({ defaultMessage: 'New Password' })}
                    field={passNew}
                />
            )}

            {passConfirm == null ? null : (
                <Field
                    name="password-new-confirm"
                    autoComplete="new-password"
                    label={formatMessage({ defaultMessage: 'Confirm New Password' })}
                    field={passConfirm}
                />
            )}
        </Form>
    );
}

interface FieldProps {
    name: string;
    label: string;
    field: iField<string>;
    autoComplete: HTMLInputAutoCompleteAttribute;
}
function Field(props: FieldProps) {
    const { name, label, autoComplete, field } = props;

    return (
        <PasswordInput
            id={$(name)}
            autoComplete={autoComplete}
            labelText={label}
            value={field.value ?? ''}
            invalid={!!field.error}
            invalidText={field.error}
            disabled={field.disabled}
            onChange={e => field.onChange(e.target.value)}
        />
    );
}
