// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { useIntl, FormattedMessage } from 'react-intl';
import { selfSelect } from '@/lib/react';
import type { iField } from '@/lib/form';

import { getID } from '../const';
import { URLS } from '@/constants';

// Components
import { Link } from '@/components';
import { TextInput } from '@carbon/react';

// Styles
import css from './forms.scss';

export interface FormBraiinsPoolProps {
    name: iField<string>;
    apiKey: iField<string>;
}

const $ = getID('braiins-pool').get;
export function FormBraiinsPool(props: FormBraiinsPoolProps) {
    const { name, apiKey } = props;

    const { formatMessage } = useIntl();
    const b = (ch: ReactNode): ReactElement => <strong key="b">{ch}</strong>;

    return (
        <div className={css.form}>
            <div className={css.intro}>
                <FormattedMessage
                    tagName="p"
                    defaultMessage="Connect your Braiins Pool account and get real-time stats for your mining operation."
                />

                <ol>
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="Open your <b>Braiins Pool account</b> and go to <a>Access Profiles</a>"
                        values={{
                            b,
                            a: ch => <Link key="a" external href={URLS.external.pool.accessProfiles} children={ch} />,
                        }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="<b>Create new Access Profile</b> with <b>Read-only permission</b>"
                        values={{ b }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="Select <b>Allow access to web APIs</b> and generate <b>API key</b>"
                        values={{ b }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="Copy your <b>Access Profile Name</b> and <b>API key</b> into the form below"
                        values={{ b }}
                    />
                </ol>
            </div>

            <div className={css.fieldWrapper}>
                <TextInput
                    id={$('name')}
                    type="string"
                    labelText={formatMessage({ defaultMessage: 'Account Name' })}
                    placeholder={formatMessage({ defaultMessage: 'Enter the access profile name' })}
                    value={name.value ?? ''}
                    onChange={e => name.onChange?.(e.target.value)}
                    onFocus={selfSelect}
                    disabled={name.disabled}
                    invalid={!!name.error}
                    invalidText={name.error}
                />
            </div>

            <div className={css.fieldWrapper}>
                <TextInput
                    id={$('api-key')}
                    type="string"
                    labelText={formatMessage({ defaultMessage: 'API Key' })}
                    placeholder={formatMessage({ defaultMessage: 'Enter the API key' })}
                    value={apiKey.value ?? ''}
                    onChange={e => apiKey.onChange?.(e.target.value)}
                    onFocus={selfSelect}
                    disabled={apiKey.disabled}
                    invalid={!!apiKey.error}
                    invalidText={apiKey.error}
                />
            </div>
        </div>
    );
}
