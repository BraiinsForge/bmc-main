import { Component, Fragment } from 'react';
import { debounce, cloneDeep, isEqual } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';

import { type IntlShape, useIntl } from 'react-intl';
import { type Location, useLocation } from 'react-router';

// Lib
import { setState } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';

// App
import AppContext, { type AppContextType } from '@/context';
import { useStore } from '@/store';
import * as pb from '@/proto';

import { SectionSettings } from './components';
import { InlineNotificationsGroup, Tabs, type TabsProps } from '@/components';
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
        const { location } = this.props;
        const maybeTabHash = location.hash.slice(1);
        if (!maybeTabHash) this.#tabChange(Tab.settings);
        else if (Object.hasOwn(Tab, maybeTabHash)) this.#tabChange(maybeTabHash as Tab);
    };

    #hasUnsavedChanges = (): boolean => {
        const { confTemp, confSaved } = this.state;
        return !isEqual(confTemp, confSaved) && !isEqual(confTemp, emptyData);
    };

    private loadAbort = pb.abort.get();
    #load = async (): Promise<void> => {
        try {
            const { signal } = this.loadAbort.replace();
            const [netInfo, netConfig] = await Promise.all([
                pb.rpc.sys.getNetworkInfo({}, { signal }),
                pb.rpc.sys.getNetworkConfig({}, { signal }),
            ]);

            const newState = {
                ...cloneDeep(this.state),
                globalErrors: null,
                confStaticErrors: null,
            };
            newState.netInfo = netInfo;
            newState.confSaved = {
                case: netConfig.protocol.case ?? 'dhcp',
                values:
                    netConfig.protocol.case === 'static'
                        ? staticConfToLocal(netConfig.protocol.value)
                        : emptyStaticConf,
            };
            newState.confTemp = cloneDeep(newState.confSaved);

            this.setState(newState);
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({ globalErrors: pb.collectAllErrors($) });
        }
    };

    private saveAbort = pb.abort.get();
    #save = async (): Promise<void> => {
        const { confSaved, confTemp } = this.state;
        const { notify } = this.context;

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

            await pb.rpc.sys.setNetworkConfig(payload, { signal });
            notify('success', 'Network configuration saved!');
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

    get #tabs(): TabsProps<Tab>['tabs'] {
        const { formatMessage } = this.props.intl;
        return [
            {
                key: Tab.settings,
                label: formatMessage({ defaultMessage: 'Settings' }),
            },
            {
                key: Tab.diagnostics,
                label: formatMessage({ defaultMessage: 'Diagnostics' }),
            },
        ];
    }
    #tabChange = (tab: Tab): void => {
        this.setState({ activeTab: tab });
        window.history.replaceState(null, '', `#${tab}`);
    };

    //
    // General
    //

    #confStaticChange = (patch: Partial<NetStaticConf>): void => {
        this.setState(s => ({
            confTemp: {
                ...s.confTemp,
                values: {
                    ...s.confTemp.values,
                    ...patch,
                },
            },
            globalErrors: null,
            confStaticErrors: null,
        }));
    };
    #confCaseChange = (v: Data['case']): void => {
        this.setState(s => ({ confTemp: { ...s.confTemp, case: v } }));
    };
    #confRender = (): ReactNode => {
        // const { formatMessage } = this.props.intl;
        const {
            netInfo,

            // Status
            isLoading,
            isSaving,

            // Validation
            globalErrors,
            confStaticErrors: err,

            // Fields
            confSaved,
            confTemp,
        } = this.state;

        const isDisabled: boolean = isLoading || isSaving;
        const hasUnsavedChanges: boolean = this.#hasUnsavedChanges();

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
                    protocol={{
                        value: confTemp.case,
                        disabled: isDisabled,
                        onChange: this.#confCaseChange,
                    }}
                    staticAddress={{
                        value: confTemp.values.address || confSaved.values.address || null,
                        error: pb.renderFieldErrorsAsList(err?.address),
                        disabled: isDisabled,
                        onChange: address => this.#confStaticChange({ address }),
                    }}
                    staticGateway={{
                        value: confTemp.values.gateway || confSaved.values.gateway || null,
                        error: pb.renderFieldErrorsAsList(err?.gateway),
                        disabled: isDisabled,
                        onChange: gateway => this.#confStaticChange({ gateway }),
                    }}
                    staticNetmask={{
                        value: confTemp.values.netmask || confSaved.values.netmask || null,
                        error: pb.renderFieldErrorsAsList(err?.netmask),
                        disabled: isDisabled,
                        onChange: netmask => this.#confStaticChange({ netmask }),
                    }}
                    staticDns={{
                        value: confTemp.values.dnsServers.length
                            ? confTemp.values.dnsServers
                            : confSaved.values.dnsServers,
                        error: pb.renderFieldErrorsAsList(err?.dnsServers),
                        disabled: isDisabled,
                        onChange: dnsServers => this.#confStaticChange({ dnsServers }),
                    }}
                    hasUnsavedChanges={hasUnsavedChanges}
                    onReset={this.#load}
                    onSave={this.#save}
                    // Wifi
                    // strings={{ wifiConnect: formatMessage({ defaultMessage: 'Connect' }) }}
                    // wifiActiveNetwork={{
                    //     value: null,
                    //     disabled: false,
                    //     error: null,
                    //     onChange: console.log.bind(console),
                    //     async onConnectionRequest() {
                    //         return false;
                    //     },
                    //     onConnectionRequestCancel: console.log.bind(console),
                    // }}
                    // wifiAvailableNetworks={{
                    //     isLoading: false,
                    //     onRefresh: console.log.bind(console),
                    //     options: [],
                    // }}
                />
            </Fragment>
        );
    };

    render() {
        const { activeTab } = this.state;
        const { title } = this.#txt;

        let content: ReactNode;
        switch (activeTab) {
            case Tab.settings:
                content = this.#confRender();
                break;

            case Tab.diagnostics:
                content = null;
                break;

            default:
                assertUnreachable(activeTab, 'settings: active tab');
        }

        return (
            <div>
                <Helmet title={title} />
                <h1 className={css.title} children={title} />

                <Tabs tabs={this.#tabs} activeTab={activeTab} onChange={this.#tabChange} className={css.tabs} />
                <div className={css.content} children={content} />
            </div>
        );
    }
}

export default function () {
    const intl = useIntl();
    const location = useLocation();
    const hasPassword = useStore(x => x.state.sessionInfo.hasPassword);
    return <View intl={intl} location={location} hasPassword={hasPassword} />;
}
