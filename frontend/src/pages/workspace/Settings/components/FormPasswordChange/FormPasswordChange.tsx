// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
            tooltipPosition="left"
            value={field.value ?? ''}
            invalid={!!field.error}
            invalidText={field.error}
            disabled={field.disabled}
            onChange={e => field.onChange?.(e.target.value)}
        />
    );
}
