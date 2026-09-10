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

import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { useNavigate, type NavigateFunction } from 'react-router';
import { Code, ConnectError } from '@connectrpc/connect';

// App, lib
import * as pb from '@/proto';
import { dnsJoin, dnsSplit } from '@/lib/format';
import { setState } from '@/lib/react';
import type { Capabilities } from '@/lib/system';
import { assertUnreachable } from '@/lib/ts';
import { useStore } from '@/store';

// Components
import { InlineNotificationsGroup } from '@/components';
import { MiningSetup, Setup } from './components/Setup';
import { awaitSetupOutcome, type FormState, isMiningSetup, postSetupDestination, toFormErrors } from './fn';

// Styles
import '@/styles/carbon/carbon.global.scss';
import css from './Init.scss';

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
    capabilities: Capabilities;
}

// NOTE: a failure to reach the server at all, as opposed to a status it sent.
function isConnectionLost(error: unknown): boolean {
    const { code } = ConnectError.from(error);
    return code === Code.Unavailable || code === Code.Unknown;
}

interface State {
    isLoading: boolean;
    isSaving: boolean;

    // Kept apart from the form errors: the form itself is held back until these clear.
    loadError: null | string[];
    data: FormState;
    timezones: Array<pb.Timezone>;
}
const getInitialState = (): State => ({
    isLoading: false,
    isSaving: false,

    loadError: null,
    data: {
        values: { protocol: 'dhcp' },
        errors: null,
    },
    timezones: [],
});

class View extends Component<Props, State> {
    readonly state = getInitialState();

    componentDidMount = () => this.#loadConfig();
    componentWillUnmount = () => pb.abort.all(this);

    private abortLoadConfig = pb.abort.get();
    #loadConfig = async (): Promise<void> => {
        const { formatMessage, timeZone } = this.props.intl;
        const { signal } = this.abortLoadConfig.replace();

        await setState(this, { isLoading: true, loadError: null });
        let timezones: Array<pb.Timezone> = [];
        let loadError: null | string[] = null;
        const res: FormState = {
            values: { protocol: 'dhcp' },
            errors: null,
        };

        try {
            const v = await pb.rpc.init.getSettingsData({}, { signal });
            timezones = v.timezones;

            // Try to detect browser timezone from react-intl first, fallback to Intl API, then server default
            const browserTimezone = timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
            const selectedTimezone =
                timezones.find(x => x.id === browserTimezone) ?? timezones.find(x => x.id === v.timezoneId);

            const staticNet = v.network?.protocol?.case === 'static' ? v.network.protocol.value : undefined;

            res.values = {
                timezone: selectedTimezone,
                timeFormat: v.timeFormat || undefined,
                dateFormat: v.dateFormat || undefined,
                numberFormat: v.numberFormat || undefined,
                temperatureUnits: v.temperatureUnit || undefined,
                unitSystem: v.unitSystem || undefined,
                // dataCollection: v.dataCollection,

                // Mining variant prefill; the backend supplies the current
                // network config, hostname, and the solo pool default for
                // miners (all null on display devices).
                // NOTE: an unset oneof (`case: undefined`) is the one value the
                // form cannot show; it is read as DHCP here, and only here.
                protocol: v.network?.protocol?.case ?? 'dhcp',
                staticAddress: staticNet?.address || undefined,
                staticNetmask: staticNet?.netmask || undefined,
                staticGateway: staticNet?.gateway || undefined,
                staticDns: staticNet ? dnsJoin(staticNet.dnsServers) || undefined : undefined,
                hostname: v.hostname || undefined,
                poolUrl: v.pool?.url || undefined,
                poolUser: v.pool?.user || undefined,
                poolPassword: v.pool?.password || undefined,
            };
        } catch ($) {
            if (pb.abort.is($)) return;
            loadError = pb.collectAllErrors($) ?? [formatMessage({ defaultMessage: 'Failed to load initial values!' })];
        }

        this.setState({ isLoading: false, loadError, timezones, data: res });
    };

    #handleChange = <Key extends keyof FormState['values']>(key: Key) => {
        return (value: FormState['values'][Key]): void => {
            this.setState(s => ({
                data: {
                    errors: null,
                    values: {
                        ...s.data.values,
                        [key]: value,
                    },
                },
            }));
        };
    };
    #getFieldError = <Key extends keyof FormState['values']>(key: Key): Maybe<string> => {
        const { errors } = this.state.data;
        return pb.renderFieldErrorsAsList(errors?.fields?.[key]);
    };

    // A miner requires the pool + network form; every other device uses the
    // localization form.
    get #isMiningSetup(): boolean {
        return isMiningSetup(this.props.capabilities);
    }

    #leaveSetup = (): void => {
        const { protocol, staticAddress } = this.state.data.values;
        const destination = postSetupDestination(this.props.capabilities, { protocol, staticAddress }, window.location);
        if ('url' in destination) window.location.replace(destination.url);
        else this.props.navigate(destination.route, { replace: true });
    };

    #buildNetworkConfig = (): pb.NetworkConfig => {
        const { protocol, staticAddress, staticNetmask, staticGateway, staticDns } = this.state.data.values;
        switch (protocol) {
            case 'static':
                return pb.create(pb.NetworkConfigSchema, {
                    protocol: {
                        case: 'static',
                        value: pb.create(pb.NetworkConfigStaticSchema, {
                            address: staticAddress ?? '',
                            netmask: staticNetmask ?? '',
                            gateway: staticGateway ?? '',
                            dnsServers: dnsSplit(staticDns ?? ''),
                        }),
                    },
                });
            case 'dhcp':
            case undefined:
                return pb.create(pb.NetworkConfigSchema, { protocol: { case: 'dhcp', value: {} } });
            default:
                return assertUnreachable(protocol, 'init-setup: network protocol');
        }
    };

    private abortSubmit = pb.abort.get();
    #submit = async (): Promise<void> => {
        const {
            // Security
            password1,
            password2,

            // Privacy
            // dataCollection,

            // Time & format
            timezone,
            dateFormat,
            numberFormat,
            timeFormat,
            temperatureUnits,
            unitSystem,

            // Mining
            poolUrl,
            poolUser,
            poolPassword,
            hostname,
        } = this.state.data.values;
        const {
            intl: { formatMessage },
        } = this.props;

        if (password1 != null && password1 !== password2) {
            this.setState(s => ({
                data: {
                    ...s.data,
                    errors: {
                        fields: {
                            password2: [formatMessage({ defaultMessage: 'Passwords have to match!' })],
                        },
                    },
                },
            }));
            return;
        }

        const request = this.#isMiningSetup
            ? pb.create(pb.SettingsRequestSchema, {
                  dateFormat,
                  numberFormat,
                  password: password1,
                  timezoneId: timezone?.id,
                  timeFormat,
                  temperatureUnit: temperatureUnits,
                  unitSystem,
                  hostname: hostname || undefined,
                  network: this.#buildNetworkConfig(),
                  pool: pb.create(pb.PoolConfigSchema, {
                      url: poolUrl ?? '',
                      user: poolUser ?? '',
                      password: poolPassword,
                  }),
              })
            : pb.create(pb.SettingsRequestSchema, {
                  // dataCollection,
                  dateFormat,
                  numberFormat,
                  password: password1,
                  timezoneId: timezone?.id,
                  timeFormat,
                  temperatureUnit: temperatureUnits,
                  unitSystem,
              });

        const { signal } = this.abortSubmit.replace();
        this.setState({ isSaving: true });
        try {
            await pb.rpc.init.setupDevice(request, { signal });
            this.#leaveSetup();
        } catch ($) {
            if (pb.abort.is($)) return;
            // NOTE: applying the network settings restarts networking on the
            // device, which can drop this very connection after the request
            // was accepted; only a device still answering as setup-pending
            // proves the request was lost. One that moved to another address
            // never answers here, so silence is taken as success.
            if (this.#isMiningSetup && isConnectionLost($)) {
                try {
                    const outcome = await awaitSetupOutcome(() => pb.rpc.init.getSettingsData({}, { signal }), {
                        signal,
                    });
                    if (outcome !== 'pending') {
                        this.#leaveSetup();
                        return;
                    }
                } catch (probeError) {
                    if (pb.abort.is(probeError)) return;
                }
            }
            const errors = toFormErrors($);
            this.setState(s => ({ isSaving: false, data: { ...s.data, errors } }));
        }
    };

    render() {
        const {
            isLoading,
            isSaving,
            loadError,
            timezones,
            data: { values, errors },
        } = this.state;
        const { formatMessage } = this.props.intl;

        const disabled: boolean = isLoading || isSaving;
        const retry = { label: formatMessage({ defaultMessage: 'Retry' }), onClick: this.#loadConfig };

        return (
            <div className={css.root}>
                <div className={css.innerSetup}>
                    <InlineNotificationsGroup
                        kind="error"
                        theme="inverse"
                        items={loadError?.map(text => ({ children: text, action: retry })) ?? errors?.global}
                        stretch
                    />
                    {loadError !== null ? null : this.#isMiningSetup ? (
                        <MiningSetup
                            timeFormat={{
                                disabled,
                                value: values.timeFormat || null,
                                error: this.#getFieldError('timeFormat'),
                                onChange: this.#handleChange('timeFormat'),
                            }}
                            timezone={{
                                value: values.timezone || null,
                                disabled,
                                items: timezones,
                                error: this.#getFieldError('timezone'),
                                onChange: this.#handleChange('timezone'),
                            }}
                            dateFormat={{
                                disabled,
                                value: values.dateFormat || null,
                                error: this.#getFieldError('dateFormat'),
                                onChange: this.#handleChange('dateFormat'),
                            }}
                            numberFormat={{
                                disabled,
                                value: values.numberFormat || null,
                                error: this.#getFieldError('numberFormat'),
                                onChange: this.#handleChange('numberFormat'),
                            }}
                            temperatureUnits={{
                                disabled,
                                value: values.temperatureUnits || null,
                                error: this.#getFieldError('temperatureUnits'),
                                onChange: this.#handleChange('temperatureUnits'),
                            }}
                            unitSystem={{
                                disabled,
                                value: values.unitSystem || null,
                                error: this.#getFieldError('unitSystem'),
                                onChange: this.#handleChange('unitSystem'),
                            }}
                            poolUrl={{
                                disabled,
                                value: values.poolUrl || null,
                                error: this.#getFieldError('poolUrl'),
                                onChange: this.#handleChange('poolUrl'),
                            }}
                            poolUser={{
                                disabled,
                                value: values.poolUser || null,
                                error: this.#getFieldError('poolUser'),
                                onChange: this.#handleChange('poolUser'),
                            }}
                            poolPassword={{
                                disabled,
                                value: values.poolPassword || null,
                                error: this.#getFieldError('poolPassword'),
                                onChange: this.#handleChange('poolPassword'),
                            }}
                            hostname={{
                                disabled,
                                value: values.hostname || null,
                                error: this.#getFieldError('hostname'),
                                onChange: this.#handleChange('hostname'),
                            }}
                            protocol={{
                                disabled,
                                value: values.protocol || 'dhcp',
                                error: this.#getFieldError('protocol'),
                                onChange: this.#handleChange('protocol'),
                            }}
                            staticAddress={{
                                disabled,
                                value: values.staticAddress || null,
                                error: this.#getFieldError('staticAddress'),
                                onChange: this.#handleChange('staticAddress'),
                            }}
                            staticNetmask={{
                                disabled,
                                value: values.staticNetmask || null,
                                error: this.#getFieldError('staticNetmask'),
                                onChange: this.#handleChange('staticNetmask'),
                            }}
                            staticGateway={{
                                disabled,
                                value: values.staticGateway || null,
                                error: this.#getFieldError('staticGateway'),
                                onChange: this.#handleChange('staticGateway'),
                            }}
                            staticDns={{
                                disabled,
                                value: values.staticDns || null,
                                error: this.#getFieldError('staticDns'),
                                onChange: this.#handleChange('staticDns'),
                            }}
                            password1={{
                                disabled,
                                value: values.password1 || null,
                                error: this.#getFieldError('password1'),
                                onChange: this.#handleChange('password1'),
                            }}
                            password2={{
                                disabled,
                                value: values.password2 || null,
                                error: this.#getFieldError('password2'),
                                onChange: this.#handleChange('password2'),
                            }}
                            onSubmit={this.#submit}
                            submitDisabled={pb.hasFormErrors(errors)}
                            submitting={isSaving}
                        />
                    ) : (
                        <Setup
                            timeFormat={{
                                disabled,
                                value: values.timeFormat || null,
                                error: this.#getFieldError('timeFormat'),
                                onChange: this.#handleChange('timeFormat'),
                            }}
                            timezone={{
                                value: values.timezone || null,
                                disabled,
                                items: timezones,
                                error: this.#getFieldError('timezone'),
                                onChange: this.#handleChange('timezone'),
                            }}
                            dateFormat={{
                                disabled,
                                value: values.dateFormat || null,
                                error: this.#getFieldError('dateFormat'),
                                onChange: this.#handleChange('dateFormat'),
                            }}
                            numberFormat={{
                                disabled,
                                value: values.numberFormat || null,
                                error: this.#getFieldError('numberFormat'),
                                onChange: this.#handleChange('numberFormat'),
                            }}
                            temperatureUnits={{
                                disabled,
                                value: values.temperatureUnits || null,
                                error: this.#getFieldError('temperatureUnits'),
                                onChange: this.#handleChange('temperatureUnits'),
                            }}
                            unitSystem={{
                                disabled,
                                value: values.unitSystem || null,
                                error: this.#getFieldError('unitSystem'),
                                onChange: this.#handleChange('unitSystem'),
                            }}
                            // Password
                            password1={{
                                disabled,
                                value: values.password1 || null,
                                error: this.#getFieldError('password1'),
                                onChange: this.#handleChange('password1'),
                            }}
                            password2={{
                                disabled,
                                value: values.password2 || null,
                                error: this.#getFieldError('password2'),
                                onChange: this.#handleChange('password2'),
                            }}
                            // // Privacy
                            // dataCollection={{
                            //     disabled,
                            //     value: values.dataCollection || null,
                            //     error: this.#getFieldError('dataCollection'),
                            //     onChange: this.#handleChange('dataCollection'),
                            // }}
                            // Form
                            onSubmit={this.#submit}
                            submitDisabled={pb.hasFormErrors(errors)}
                            submitting={isSaving}
                        />
                    )}
                </div>
            </div>
        );
    }
}

export default function InitSetup() {
    const intl = useIntl();
    const navigate = useNavigate();
    const capabilities = useStore(x => x.state.hardwareCapabilities);
    return <View intl={intl} navigate={navigate} capabilities={capabilities} />;
}
