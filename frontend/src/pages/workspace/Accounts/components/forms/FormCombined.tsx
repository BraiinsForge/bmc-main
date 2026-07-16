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

import { FormattedMessage, useIntl } from 'react-intl';

// Libs
import { type iField, Form } from '@/lib/form';
import { assertUnreachable } from '@/lib/ts';

// App
import * as pb from '@/proto';
import { getID } from '../const';

// Components
import { FormBraiinsPool, type FormBraiinsPoolProps } from './FormBraiinsPool';
import { Select, SelectItem } from '@carbon/react';

// Styles
import cn from 'clsx';
import css from './forms.scss';

export interface FormCombinedProps {
    type: iField<pb.AccountType>;
    valuesBraiinsPool: FormBraiinsPoolProps;
    connectedWidgetsCount: null | number;
}

const $ = getID('combined').get;

export function FormCombined(props: FormCombinedProps) {
    const { type, valuesBraiinsPool, connectedWidgetsCount } = props;

    const intl = useIntl();
    const { formatMessage } = intl;

    let typeForm: ReactNode;
    switch (type.value) {
        case null:
        case undefined:
        case pb.AccountType.UNSPECIFIED:
            break;

        case pb.AccountType.BRAIINSPOOL:
            typeForm = <FormBraiinsPool {...valuesBraiinsPool} />;
            break;

        default:
            assertUnreachable(type.value, 'Unknown account type');
    }

    return (
        <Form className={css.form}>
            <div className={css.fieldWrapper}>
                <Select
                    id={$('name')}
                    labelText={formatMessage({ defaultMessage: 'Type' })}
                    value={type.value ?? ''}
                    onChange={e => type.onChange?.(e.target.value as unknown as pb.AccountType)}
                    disabled={type.disabled}
                    invalid={!!type.error}
                    invalidText={type.error}
                    readOnly={pb.accountTypeOptions.length <= 1}
                    children={pb.accountTypeOptions.map(x => (
                        <SelectItem key={x} value={x} text={pb.accountTypeToString(intl, x) ?? 'N/A'} />
                    ))}
                />
            </div>

            {typeForm}

            {connectedWidgetsCount ? (
                <div className={cn(css.withIcon, css.dimmed)}>
                    <FormattedMessage
                        defaultMessage="Connected to {count, plural, one {1 display widget} other {# display widgets}}"
                        values={{ count: connectedWidgetsCount }}
                    />
                </div>
            ) : null}
        </Form>
    );
}
