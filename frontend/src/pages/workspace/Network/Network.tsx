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

import { Component, Fragment } from 'react';
import { debounce, cloneDeep, isEqual } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';

import { type IntlShape, useIntl } from 'react-intl';
import { type Location, useLocation } from 'react-router';

// Lib
import { setState } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';
import { toast } from '@/lib/toast';

// App
import AppContext, { type AppContextType } from '@/context';
import { useStore } from '@/store';
import * as pb from '@/proto';

import { SectionSettings } from './components';
import {
    InlineNotificationsGroup,
    // Tabs
    // type TabsProps,
} from '@/components';
import css from './Network.scss';

interface Props {
    intl: IntlShape;
    location: Location;
    hasPassword: null | boolean;
}

// The value is used in location hash, so be mindfull of that.
enum Tab {
    settings = 'settings',
    diagnostics = 'diagnostics',
}

type NetProto = NonNullable<pb.NetworkConfig['protocol']['case']>;
type NetStaticConf = Omit<pb.NetworkConfigStatic, 'dnsServers'> & { dnsServers: string };
const emptyStaticConf: Readonly<NetStaticConf> = Object.freeze({
    $typeName: 'braiins.bmc.web.NetworkConfigStatic',
    gateway: '',
    netmask: '',
    address: '',
    dnsServers: '',
} satisfies NetStaticConf);

type Data = {
    case: NetProto;
    values: NetStaticConf;
};
const emptyData: Readonly<Data> = Object.freeze({
    case: 'dhcp',
    values: emptyStaticConf,
} satisfies Data);

function dnsJoin(value: string[]): string {
    return value.join(', ');
}
function dnsSplit(value: string): string[] {
    return value
        .split(',')
        .map(x => x.trim())
        .filter(Boolean);
}
function staticConfToLocal(value: pb.NetworkConfigStatic): NetStaticConf {
    const { dnsServers, ...rest } = value;
    return { ...rest, dnsServers: dnsJoin(dnsServers) };
}

interface State {
    activeTab: Tab;

    isLoading: boolean;
    isSaving: boolean;

    netInfo: pb.NetworkInfoResponse;
    wifi: {
        error: null | string;
        status: null | pb.WifiStatusResponse;
        savedNets: null | pb.WifiSavedNetworksResponse;

        isScanning: boolean;
        nets: pb.WifiNetwork[];

        selection: null | pb.WifiNetwork;
    };

    confSaved: Data;
    confTemp: Data;

    globalErrors: Maybe<string[]>;
    confStaticErrors: null | pb.FormErrors<NetStaticConf>['fields'];
}
const getInitialState = (): State => ({
    activeTab: Tab.settings,
    isLoading: false,
    isSaving: false,

    netInfo: pb.create(pb.NetworkInfoResponseSchema),
    wifi: {
        error: null,

        status: null,
        savedNets: null,

        isScanning: false,
        nets: [],

        selection: null,
    },

    confSaved: emptyData,
    confTemp: emptyData,

    globalErrors: null,
    confStaticErrors: null,
});

class View extends Component<Props, State> {
    readonly state = getInitialState();
    static contextType = AppContext;
    declare context: AppContextType;

    componentDidMount = () => this.#mount();
    componentWillUnmount = () => pb.abort.all(this);

    #mount = debounce(() => {
        this.#syncTabs();
        this.#load();
    }, 150);
    #syncTabs = () => {
        // const { location } = this.props;
        // const maybeTabHash = location.hash.slice(1);
        // if (!maybeTabHash) this.#tabChange(Tab.settings);
        // else if (Object.hasOwn(Tab, maybeTabHash)) this.#tabChange(maybeTabHash as Tab);
    };

    #hasUnsavedChanges = (): boolean => {
        const { confTemp, confSaved } = this.state;
        return !isEqual(confTemp, confSaved) && !isEqual(confTemp, emptyData);
    };

    private loadAbort = pb.abort.get();
    #load = async (): Promise<void> => {
        try {
            const { signal } = this.loadAbort.replace();
            const reqOpt = { signal };

            const [netInfo, netConfig, wifiStatus, wifiSavedNets] = await Promise.all([
                pb.rpc.net.getNetworkInfo({}, reqOpt),
                pb.rpc.net.getNetworkConfig({}, reqOpt),
                pb.rpc.net.getWifiStatus({}, reqOpt),
                pb.rpc.net.getWifiSavedNetworks({}, reqOpt),
            ]);

            this.setState(s => {
                const newState = {
                    ...cloneDeep(s),
                    globalErrors: null,
                    confStaticErrors: null,
                };

                newState.netInfo = netInfo;
                newState.wifi = {
                    ...newState.wifi,
                    status: wifiStatus,
                    savedNets: wifiSavedNets,
                    selection:
                        newState.wifi.selection == null
                            ? (wifiStatus.status?.network ?? null)
                            : newState.wifi.selection,
                };

                newState.confSaved = {
                    case: netConfig.protocol.case ?? 'dhcp',
                    values:
                        netConfig.protocol.case === 'static'
                            ? staticConfToLocal(netConfig.protocol.value)
                            : emptyStaticConf,
                };
                newState.confTemp = cloneDeep(newState.confSaved);

                return newState;
            });
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({ globalErrors: pb.collectAllErrors($) });
        }
    };

    private wifiScanAbort = pb.abort.get();
    #wifiScan = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.wifiScanAbort.replace();

        await setState(this, s => ({ wifi: { ...s.wifi, isScanning: true } }));

        try {
            const response = await pb.rpc.net.scanWifi({}, { signal });
            this.setState(s => ({
                wifi: {
                    ...s.wifi,
                    isScanning: false,
                    nets: response.networks,
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Wi-Fi scan: Unknown error!' });
            this.setState(s => ({
                wifi: {
                    ...s.wifi,
                    error: msg,
                    isScanning: false,
                },
            }));
        }
    };

    #wifiChange = (value: pb.WifiNetwork): void => {
        this.setState(s => ({
            wifi: {
                ...s.wifi,
                selection: value,
            },
        }));
    };
    private wifiConnectAbort = pb.abort.get();
    #wifiConnect = async (ssid: string, encryptionType: pb.EncryptionType, password: string): Promise<boolean> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.wifiConnectAbort.replace();

        try {
            await pb.rpc.net.setWifi(pb.create(pb.SetWifiRequestSchema, { ssid, encryptionType, password }), {
                signal,
            });
            return true;
        } catch ($) {
            if (pb.abort.is($)) return false;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Wi-Fi connection: Unknown error!' });
            toast.error(msg);
            return false;
        }
    };
    #wifiConnectCancel = (): void => this.wifiConnectAbort.abort();

    private saveAbort = pb.abort.get();
    #save = async (): Promise<void> => {
        const { confSaved, confTemp } = this.state;

        const hasUnsavedChanges: boolean = this.#hasUnsavedChanges();
        if (!hasUnsavedChanges) return;
        await setState(this, { isSaving: true, globalErrors: null, confStaticErrors: null });

        try {
            const { signal } = this.saveAbort.replace();

            const payload = pb.create(pb.NetworkConfigSchema);
            switch (confTemp.case) {
                case 'dhcp':
                    payload.protocol.case = 'dhcp';
                    break;

                case 'static':
                    payload.protocol.case = 'static';
                    payload.protocol.value = pb.create(pb.NetworkConfigStaticSchema, {
                        address: confTemp.values.address || confSaved.values.address,
                        netmask: confTemp.values.netmask || confSaved.values.netmask,
                        gateway: confTemp.values.gateway || confSaved.values.gateway,
                        dnsServers: dnsSplit(
                            confTemp.values.dnsServers.length
                                ? confTemp.values.dnsServers
                                : confSaved.values.dnsServers,
                        ),
                    });
                    break;

                default:
                    assertUnreachable(confTemp.case, 'settings: confCase');
            }

            await pb.rpc.net.setNetworkConfig(payload, { signal });
            toast.success('Network configuration saved!');
            await this.#load();
        } catch ($) {
            if (pb.abort.is($)) return;

            const newState: Pick<State, 'globalErrors' | 'confStaticErrors' | 'isSaving'> = {
                confStaticErrors: null,
                globalErrors: null,
                isSaving: false,
            };
            switch (confTemp.case) {
                case 'dhcp':
                    newState.globalErrors = pb.collectAllErrors($);
                    break;

                case 'static': {
                    const { global, fields } = pb.parseFormErrors<pb.NetworkConfigStatic>(
                        $,
                        Object.keys(emptyStaticConf),
                    );

                    newState.globalErrors = global;
                    newState.confStaticErrors = {
                        ...fields,
                        dnsServers: fields.dnsServers?.flat(),
                    };
                    break;
                }

                default:
                    assertUnreachable(confTemp.case, 'settings: confCase');
            }

            this.setState(newState);
        }

        this.setState({ isSaving: false });
    };

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Network Configuration' }),
        };
    }

    // get #tabs(): TabsProps<Tab>['tabs'] {
    //     const { formatMessage } = this.props.intl;
    //     return [
    //         {
    //             key: Tab.settings,
    //             label: formatMessage({ defaultMessage: 'Settings' }),
    //         },
    //         {
    //             key: Tab.diagnostics,
    //             label: formatMessage({ defaultMessage: 'Diagnostics' }),
    //             disabled: true,
    //         },
    //     ];
    // }
    // #tabChange = (tab: Tab): void => {
    //     this.setState({ activeTab: tab });
    //     window.history.replaceState(null, '', `#${tab}`);
    // };

    //
    // General
    //

    // #confStaticChange = (patch: Partial<NetStaticConf>): void => {
    //     this.setState(s => ({
    //         confTemp: {
    //             ...s.confTemp,
    //             values: {
    //                 ...s.confTemp.values,
    //                 ...patch,
    //             },
    //         },
    //         globalErrors: null,
    //         confStaticErrors: null,
    //     }));
    // };
    // #confCaseChange = (v: Data['case']): void => {
    //     this.setState(s => ({ confTemp: { ...s.confTemp, case: v } }));
    // };
    #confRender = (): ReactNode => {
        const { formatMessage } = this.props.intl;
        const {
            netInfo,
            wifi,

            // Status
            // isLoading,
            // isSaving,

            // Validation
            globalErrors,
            // confStaticErrors: err,

            // Fields
            // confSaved,
            // confTemp,
        } = this.state;

        // const isDisabled: boolean = isLoading || isSaving;
        const hasUnsavedChanges: boolean = this.#hasUnsavedChanges();

        const wifiNetworks: Map<pb.WifiNetwork['ssid'], pb.WifiNetwork> = new Map(wifi.nets.map(x => [x.ssid, x]));
        if (wifi.savedNets) {
            wifi.savedNets.status.forEach(({ network }) => {
                if (!network) return;
                wifiNetworks.set(network.ssid, network);
            });
        }

        return (
            <Fragment>
                <InlineNotificationsGroup
                    kind="error"
                    items={globalErrors}
                    stretch
                    theme="inverse"
                    style={{ marginBottom: '1rem' }}
                />

                <SectionSettings
                    status={[
                        ['IPv4', netInfo.ipAddress],
                        ['Hostname', netInfo.hostname],
                        ['MAC Address', netInfo.macAddress],
                    ]}
                    // Currently only shown in the status section
                    hostname={null}
                    // protocol={{
                    //     value: confTemp.case,
                    //     disabled: isDisabled,
                    //     onChange: this.#confCaseChange,
                    // }}
                    // staticAddress={{
                    //     value: confTemp.values.address || confSaved.values.address || null,
                    //     error: pb.renderFieldErrorsAsList(err?.address),
                    //     disabled: isDisabled,
                    //     onChange: address => this.#confStaticChange({ address }),
                    // }}
                    // staticGateway={{
                    //     value: confTemp.values.gateway || confSaved.values.gateway || null,
                    //     error: pb.renderFieldErrorsAsList(err?.gateway),
                    //     disabled: isDisabled,
                    //     onChange: gateway => this.#confStaticChange({ gateway }),
                    // }}
                    // staticNetmask={{
                    //     value: confTemp.values.netmask || confSaved.values.netmask || null,
                    //     error: pb.renderFieldErrorsAsList(err?.netmask),
                    //     disabled: isDisabled,
                    //     onChange: netmask => this.#confStaticChange({ netmask }),
                    // }}
                    // staticDns={{
                    //     value: confTemp.values.dnsServers.length
                    //         ? confTemp.values.dnsServers
                    //         : confSaved.values.dnsServers,
                    //     error: pb.renderFieldErrorsAsList(err?.dnsServers),
                    //     disabled: isDisabled,
                    //     onChange: dnsServers => this.#confStaticChange({ dnsServers }),
                    // }}
                    hasUnsavedChanges={hasUnsavedChanges}
                    onReset={this.#load}
                    onSave={this.#save}
                    // Wifi
                    strings={{ wifiConnect: formatMessage({ defaultMessage: 'Connect' }) }}
                    wifiActiveNetwork={{
                        value: wifi.selection,
                        disabled: wifi.isScanning,
                        error: wifi.error,
                        onChange: this.#wifiChange,
                        onConnectionRequest: this.#wifiConnect,
                        onConnectionRequestCancel: this.#wifiConnectCancel,
                    }}
                    wifiAvailableNetworks={{
                        isLoading: wifi.isScanning,
                        onRefresh: this.#wifiScan,
                        options: Array.from(wifiNetworks.values()),
                    }}
                />
            </Fragment>
        );
    };

    render() {
        // const { activeTab } = this.state;

        const { title } = this.#txt;
        const content = this.#confRender();

        // let content: ReactNode;
        // switch (activeTab) {
        //     case Tab.settings:
        //         content = this.#confRender();
        //         break;
        //
        //     case Tab.diagnostics:
        //         content = null;
        //         break;
        //
        //     default:
        //         assertUnreachable(activeTab, 'settings: active tab');
        // }

        return (
            <div className={css.root}>
                <Helmet title={title} />
                <h1 className={css.title} children={title} />

                {/* <Tabs tabs={this.#tabs} activeTab={activeTab} onChange={this.#tabChange} className={css.tabs} /> */}
                <div className={css.content} children={content} />
            </div>
        );
    }
}

export default function NetworkPage() {
    const intl = useIntl();
    const location = useLocation();
    const hasPassword = useStore(x => x.state.sessionInfo.hasPassword);
    return <View intl={intl} location={location} hasPassword={hasPassword} />;
}
