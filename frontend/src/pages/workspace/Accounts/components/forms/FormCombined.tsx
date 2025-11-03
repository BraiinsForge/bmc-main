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
