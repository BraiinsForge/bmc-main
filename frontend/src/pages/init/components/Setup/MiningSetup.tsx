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

import { Fragment, type ReactElement } from 'react';
import { useIntl } from 'react-intl';

import type * as pb from '@/proto';
import { Form, getID, type iField } from '@/lib/form';

// Components
import { Layout } from '../Layout';
import { ButtonSwitch, FieldSet, Field, LogoHeaderMiner, Button } from '@/components';
import { PasswordInput, ProgressIndicator, ProgressStep, TextInput } from '@carbon/react';
import {
    LocalizationFields,
    type LocalizationFieldsProps,
    PasswordFields,
    type PasswordFieldsProps,
    catchEscapeKey,
} from './SetupFields';

// Styles
import css from './Setup.scss';

export type NetworkProtocol = NonNullable<pb.NetworkConfig['protocol']['case']>;

export interface MiningSetupProps extends LocalizationFieldsProps, PasswordFieldsProps {
    // Pool
    poolUrl: iField<string>;
    poolUser: iField<string>;
    poolPassword: iField<string>;

    // Network
    hostname: iField<string>;
    protocol: iField<NetworkProtocol>;
    staticAddress: iField<string>;
    staticNetmask: iField<string>;
    staticGateway: iField<string>;
    staticDns: iField<string>;

    onSubmit(): void;
    submitDisabled?: boolean;
    submitting?: boolean;
}

const $ = getID('initial-setup-mining').get;

function TextField(props: { field: iField<string>; id: string; label: string; placeholder?: string }): ReactElement {
    const { field, id, label, placeholder } = props;
    return (
        <Field variant="light" title={label} disabled={field.disabled}>
            <TextInput
                id={$(id)}
                labelText=""
                hideLabel
                placeholder={placeholder}
                value={field.value ?? ''}
                onChange={e => field.onChange?.(e.target.value)}
                disabled={field.disabled}
                invalid={!!field.error}
                invalidText={field.error}
            />
        </Field>
    );
}

function PasswordField(props: { field: iField<string>; id: string; label: string }): ReactElement {
    const { field, id, label } = props;
    return (
        <Field variant="light" title={label} disabled={field.disabled}>
            <PasswordInput
                id={$(id)}
                hideLabel
                labelText={null}
                tooltipPosition="left"
                value={field.value ?? ''}
                onChange={e => field.onChange?.(e.target.value)}
                disabled={field.disabled}
                invalid={!!field.error}
                invalidText={field.error}
                placeholder="---"
            />
        </Field>
    );
}

/** Miner (BMM101) setup: mining pool and network on top of the shared fields. */
export function MiningSetup(props: MiningSetupProps) {
    const { formatMessage } = useIntl();
    const {
        onSubmit,
        submitDisabled,
        submitting,
        poolUrl,
        poolUser,
        poolPassword,
        hostname,
        protocol,
        staticAddress,
        staticNetmask,
        staticGateway,
        staticDns,
        ...fields
    } = props;

    return (
        <Layout
            header={<LogoHeaderMiner style={{ width: 'auto', height: 18 }} />}
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
                    defaultMessage: 'Configure your mining pool and network to get your miner online.',
                })}
            />

            <Form className={css.form} onKeyDownCapture={catchEscapeKey}>
                <FieldSet title={formatMessage({ defaultMessage: 'Mining Pool' })}>
                    <TextField
                        field={poolUrl}
                        id="pool-url"
                        label={formatMessage({ defaultMessage: 'URL' })}
                        placeholder="stratum+tcp://…"
                    />
                    <TextField field={poolUser} id="pool-user" label={formatMessage({ defaultMessage: 'User' })} />
                    <PasswordField
                        field={poolPassword}
                        id="pool-password"
                        label={formatMessage({ defaultMessage: 'Password' })}
                    />
                </FieldSet>

                <FieldSet
                    title={formatMessage({ defaultMessage: 'Ethernet Network' })}
                    description={formatMessage({
                        defaultMessage: 'The DHCP or static address configuration applies to the ethernet port.',
                    })}
                >
                    <TextField field={hostname} id="hostname" label={formatMessage({ defaultMessage: 'Hostname' })} />

                    <Field
                        variant="light"
                        title={formatMessage({ defaultMessage: 'Protocol' })}
                        disabled={protocol.disabled}
                    >
                        <ButtonSwitch<NetworkProtocol>
                            id={$('protocol')}
                            selectedOption={protocol.value}
                            options={[
                                { id: 'dhcp', text: formatMessage({ defaultMessage: 'DHCP' }) },
                                { id: 'static', text: formatMessage({ defaultMessage: 'Static' }) },
                            ]}
                            disabled={protocol.disabled}
                            onChange={protocol.onChange}
                            invalid={!!protocol.error}
                            invalidText={protocol.error}
                        />
                    </Field>

                    {protocol.value === 'static' ? (
                        <Fragment>
                            <TextField
                                field={staticAddress}
                                id="address"
                                label={formatMessage({ defaultMessage: 'IP Address' })}
                                placeholder="0.0.0.0"
                            />
                            <TextField
                                field={staticNetmask}
                                id="netmask"
                                label={formatMessage({ defaultMessage: 'Netmask' })}
                                placeholder="0.0.0.0"
                            />
                            <TextField
                                field={staticGateway}
                                id="gateway"
                                label={formatMessage({ defaultMessage: 'Gateway' })}
                                placeholder="0.0.0.0"
                            />
                            <TextField
                                field={staticDns}
                                id="dns"
                                label={formatMessage({ defaultMessage: 'DNS Servers' })}
                                placeholder="1.1.1.1, 8.8.8.8"
                            />
                        </Fragment>
                    ) : null}
                </FieldSet>

                <LocalizationFields $={$} {...fields} />
                <PasswordFields $={$} {...fields} />
            </Form>
        </Layout>
    );
}
