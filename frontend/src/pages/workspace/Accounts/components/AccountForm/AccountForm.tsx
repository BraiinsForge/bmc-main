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

import { Form, type iField } from '@/lib/form';
import { selfSelect } from '@/lib/react';

import type * as pb from '@/proto';
import { getID } from '../const';

import { AccountIcon, type FieldValue, InlineNotification } from '@/components';
import { CredentialTypeForm } from '../CredentialTypeForm';
import { RadioButton, RadioButtonGroup, TextArea, TextInput } from '@carbon/react';
import css from './AccountForm.scss';

export interface AccountFormProps {
    mode: 'create' | 'edit';
    credentialTypes: pb.CredentialTypeLookup;

    type: iField<string>;
    name: iField<string>;

    // The credential type's secret fields are dynamic, so they stay a controlled value map rather
    // than a static `iField` per key.
    fieldValues: Record<string, FieldValue>;
    fieldErrors?: Record<string, string[] | undefined>;
    onFieldChange(key: string, value: FieldValue): void;

    // Raw textarea text, one destination per line; the parent splits it on save.
    // Only rendered for a type without its own egress pin.
    allowHosts: iField<string>;

    // Top-level error (a non-field failure).
    error?: null | string;
}

const $ = getID('account-form').get;
const br: ReactNode = <br />;

// Fully controlled, presentational account form: the credential-type picker, the account name, the
// secret-disclosure notice, and the type's masked fields. The parent owns all state and submission.
export function AccountForm(props: AccountFormProps) {
    const { mode, credentialTypes, type, name, fieldValues, fieldErrors, onFieldChange, allowHosts, error } = props;
    const { formatMessage } = useIntl();

    const isEdit = mode === 'edit';
    const selectedType = type.value ? credentialTypes.get(type.value) : undefined;
    const typePin = selectedType?.egress?.allowHosts ?? [];

    return (
        <Form className={css.form}>
            {error ? (
                <InlineNotification stretch theme="inverse" kind="error" hideCloseButton children={error} />
            ) : null}

            <RadioButtonGroup
                // Remount on value change: the group otherwise keeps
                // a stale checked state when the controlled value changes
                // externally (same workaround as the display size selector).
                key={`${$('type')}-${type.value ?? ''}`}
                name={$('type')}
                legendText={formatMessage({ defaultMessage: 'Type' })}
                orientation="vertical"
                value={type.value ?? undefined}
                onChange={value => type.onChange?.(String(value))}
                invalid={!!type.error}
                invalidText={type.error}
                children={Array.from(credentialTypes.values(), t => (
                    <RadioButton
                        key={t.id}
                        value={t.id}
                        checked={type.value === t.id}
                        // Edit locks the type: keep the selected radio live (bright), disable the rest.
                        disabled={type.disabled || (isEdit && type.value !== t.id)}
                        labelText={
                            <div className={css.radioLabel}>
                                <AccountIcon size={20} icon={t.icon} />
                                <span children={t.name} />
                            </div>
                        }
                    />
                ))}
            />

            <TextInput
                id={$('name')}
                type="text"
                labelText={formatMessage({ defaultMessage: 'Account Name' })}
                value={name.value ?? ''}
                onChange={e => name.onChange?.(e.target.value)}
                onFocus={selfSelect}
                disabled={name.disabled}
                invalid={!!name.error}
                invalidText={name.error}
            />

            {selectedType ? (
                <CredentialTypeForm
                    type={selectedType}
                    values={fieldValues}
                    errors={fieldErrors}
                    onChange={onFieldChange}
                />
            ) : null}

            {selectedType && typePin.length === 0 ? (
                <TextArea
                    id={$('allow-hosts')}
                    rows={3}
                    labelText={formatMessage({ defaultMessage: 'Allowed destinations (optional)' })}
                    helperText={formatMessage(
                        {
                            defaultMessage:
                                'One per line: host / "*.example.com" wildcard / CIDR range.{br}Leave empty to allow any destination.',
                        },
                        { br },
                    )}
                    placeholder={'api.example.com\n*.example.com\n10.0.0.0/8'}
                    value={allowHosts.value ?? ''}
                    onChange={e => allowHosts.onChange?.(e.target.value)}
                    disabled={allowHosts.disabled}
                    invalid={!!allowHosts.error}
                    invalidText={allowHosts.error}
                />
            ) : null}
        </Form>
    );
}
