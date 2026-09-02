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

import { Form, getID } from '@/lib/form';

// Components
import { Layout } from '../Layout';
import { LogoHeader, Button } from '@/components';
import { ProgressIndicator, ProgressStep } from '@carbon/react';
import {
    LocalizationFields,
    type LocalizationFieldsProps,
    PasswordFields,
    type PasswordFieldsProps,
    catchEscapeKey,
} from './SetupFields';

// Styles
import css from './Setup.scss';

export interface SetupProps extends LocalizationFieldsProps, PasswordFieldsProps {
    // dataCollection: iField<boolean>;

    onSubmit(): void;
    submitDisabled?: boolean;
    submitting?: boolean;
}

const $ = getID('initial-setup-profile').get;

/** Display device (Deck) setup: localization and the optional password. */
export function Setup(props: SetupProps) {
    const { formatMessage } = useIntl();
    const { onSubmit, submitDisabled, submitting, ...fields } = props;

    return (
        <Layout
            header={<LogoHeader style={{ width: 'auto', height: 18 }} />}
            footer={[
                <span key="a" />,
                <Button
                    id={$('save-and-continue')}
                    key="b"
                    kind="primary"
                    disabled={submitDisabled}
                    loading={submitting}
                    onClick={onSubmit}
                    children={formatMessage({ defaultMessage: 'Save and Continue' })}
                />,
            ]}
            className={css.layout}
        >
            <ProgressIndicator currentIndex={1} className={css.progress}>
                <ProgressStep label="Wi-Fi Settings" />
                <ProgressStep label="Initial Setup" className={css.disabledTab} />
            </ProgressIndicator>

            <h1 className={css.title} children={formatMessage({ defaultMessage: 'Device Setup' })} />
            <p
                className={css.note}
                children={formatMessage({
                    defaultMessage:
                        'Configure essential settings like time, network, and access to prepare your clock for use.',
                })}
            />

            <Form className={css.form} onKeyDownCapture={catchEscapeKey}>
                <LocalizationFields $={$} {...fields} />
                <PasswordFields $={$} {...fields} />

                {/*
                <FieldSet title={formatMessage({ defaultMessage: 'Usage Data' })}>
                    <Field
                        variant="light"
                        title={formatMessage({ defaultMessage: 'Data Collection' })}
                        description={formatMessage({
                            defaultMessage: 'Allow anonymous data collection to improve the product',
                        })}
                        disabled={dataCollection.disabled}
                    >
                        <Toggle
                            id={$('data-collection')}
                            toggled={!!dataCollection.value}
                            onToggle={dataCollection.onChange}
                            disabled={dataCollection.disabled}
                        />
                    </Field>
                </FieldSet>
                */}
            </Form>
        </Layout>
    );
}
