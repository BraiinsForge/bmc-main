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
import type * as pb from '@/proto';
import css from './CredentialTypeForm.scss';
import { Markdown, ParamField, type FieldValue, InlineNotification } from '@/components';

export interface CredentialTypeFormProps {
    type: pb.CredentialType;
    values: Record<string, FieldValue>;
    onChange(key: string, value: FieldValue): void;
    // Per-field errors keyed by field key (from the backend's `field_values["<key>"]` violations).
    errors?: Record<string, string[] | undefined>;
}

// Renders a credential type's fields via the shared ParamField; secret fields mask via the Password
// format. Controlled — the parent owns the field values and submits them.
export function CredentialTypeForm({ type, values, onChange, errors }: CredentialTypeFormProps) {
    const { formatMessage } = useIntl();

    // Derived from the structured policy rather than the type's own prose,
    // so it stays truthful for a type whose description someone else wrote.
    const hosts = type.egress?.allowHosts ?? [];
    const egress =
        hosts.length > 0
            ? formatMessage({ defaultMessage: 'It is only ever sent to {hosts}.' }, { hosts: hosts.join(', ') })
            : formatMessage({ defaultMessage: 'It may be sent to any host.' });

    return (
        <div className={css.root}>
            <div className={css.intro}>
                <h4 className={css.title} children={formatMessage({ defaultMessage: 'Credentials' })} />
                <Markdown className={css.description} source={type.description} />
                <InlineNotification
                    theme="inverse"
                    kind="info"
                    stretch
                    hideCloseButton
                    className={css.egress}
                    children={egress}
                />
            </div>

            <section
                children={type.fields.map(field => (
                    <ParamField
                        key={field.key}
                        id={`cred-${type.id}-${field.key}`}
                        definition={field}
                        value={values[field.key] ?? ''}
                        error={errors?.[field.key]?.[0]}
                        onChange={onChange}
                        timezones={[]}
                    />
                ))}
            />
        </div>
    );
}
