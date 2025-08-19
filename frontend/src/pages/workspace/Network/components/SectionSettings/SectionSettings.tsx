import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

// Lib
import { AutoSelected, selfSelect, setState } from '@/lib/react';
import { Form, type iField, getID } from '@/lib/form';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import {
    Field,
    FieldSet,
    Button,
    Modal,
    WifiNetworkLine,
    // ButtonSwitch
} from '@/components';
import { TextInput, InlineLoading, Dropdown, Select, SelectItem, PasswordInput } from '@carbon/react';
import { Renew as IconRefresh } from '@carbon/react/icons';

// Styles
// import cn from 'clsx';
import css from './SectionSettings.scss';

// type NetProto = NonNullable<pb.NetworkConfig['protocol']['case']>;

export interface SectionSettingsProps {
    status: Array<[label: ReactNode, value: ReactNode]>;

    hostname: null | iField<string>;
    // protocol: iField<NetProto>;
    // staticAddress: iField<string>;
    // staticNetmask: iField<string>;
    // staticGateway: iField<string>;
    // staticDns: iField<string>;

    // Wifi
    // The connected bool attribute is remapped to nullability
    strings: { wifiConnect: string };
    wifiActiveNetwork: iField<Maybe<pb.WifiNetwork>> & {
        onConnectionRequest(ssid: string, security: pb.EncryptionType, password: string): Promise<boolean>;
        onConnectionRequestCancel?(): void;
    };
    wifiAvailableNetworks: {
        isLoading: boolean;
        options: pb.WifiNetwork[];
        onRefresh(): void;
    };

    hasUnsavedChanges: boolean;
    onSave(): void;
    onReset(): void;
}
interface Props extends SectionSettingsProps {
    intl: IntlShape;
}

/**
 * The selection dropdown is a Downshift instance,
 * which requires all options to be of the same type.
 *
 * This is a placeholder object that represents the "Other…" option.
 * It needs to be caught and handled in special way in…
 *  - renderToString
 *  - renderToElement
 *  - onChange
 */
export const WIFI_AP_OTHER_PLACEHOLDER: Readonly<pb.WifiNetwork> = Object.freeze({
    $typeName: 'braiins.bmc.web.WifiNetwork',
    ssid: 'WIFI_AP_OTHER_PLACEHOLDER',
    macAddress: 'WIFI_AP_OTHER_PLACEHOLDER',
    connected: false,
    encryptionType: pb.EncryptionType.WPA,
    signalStrength: pb.SignalStrength.UNSPECIFIED,
});

interface ManualWifiApConfig {
    ssid: string;
    security: pb.EncryptionType;
    password: string;
}
interface State {
    openDialog: null | 'wifiManualConnect' | 'wifiPasswordEntry';

    wifiManualConnect: {
        data: ManualWifiApConfig;
        errors: null | Partial<Record<keyof ManualWifiApConfig | 'form', string>>;
    };
    wifiPasswordEntry: {
        wifiCell: null | pb.WifiNetwork;
        password: string;
        passwordError: null | string;
    };
    wifiConnectionError: null | string;
    wifiNetworkDropdownKey: number;
    wifiIsConnecting: boolean;
}
const getInitialState = (): State => ({
    openDialog: null,

    wifiManualConnect: {
        data: {
            ssid: '',
            security: pb.EncryptionType.UNSPECIFIED,
            password: '',
        },
        errors: null,
    },
    wifiPasswordEntry: {
        wifiCell: null,
        password: '',
        passwordError: null,
    },
    wifiConnectionError: null,
    wifiNetworkDropdownKey: 0,
    wifiIsConnecting: false,
});

const $ = getID('settings', 'network', 'config').get;
class View extends Component<Props, State> {
    readonly state = getInitialState();
    static contextType = AppContext;
    declare context: AppContextType;

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            security: formatMessage({ defaultMessage: 'Security' }),
            network: formatMessage({ defaultMessage: 'Network' }),
            password: formatMessage({ defaultMessage: 'Password' }),

            // Action words
            refresh: formatMessage({ defaultMessage: 'Refresh' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            join: formatMessage({ defaultMessage: 'Join' }),

            // Validation
            requiredField: formatMessage({ defaultMessage: 'This field is required' }),
            passwordIsRequired: formatMessage({ defaultMessage: 'Password is required' }),

            // Wifi
            wifi: formatMessage({ defaultMessage: 'Wi-Fi' }),
            ssid: formatMessage({ defaultMessage: 'SSID' }),
            other: formatMessage({ defaultMessage: 'Other…' }),
            selectWifiNetwork: formatMessage({ defaultMessage: 'Select a Wi-Fi Network…' }),

            connecting: formatMessage({ defaultMessage: 'Connecting…' }),
            connectingElement: (
                <InlineLoading status="active" description={formatMessage({ defaultMessage: 'Connecting…' })} />
            ),

            scanningElement: (
                <InlineLoading
                    status="active"
                    description={formatMessage({ defaultMessage: 'Scanning for networks…' })}
                />
            ),
        };
    }

    #renderStats = (): ReactNode => {
        const { status } = this.props;
        return (
            <div
                className={css.status}
                children={status.map(([label, value], i): ReactNode => {
                    return (
                        <div key={i} className={css.statusRow}>
                            <strong children={label} />
                            <AutoSelected kind="span" children={value} />
                        </div>
                    );
                })}
            />
        );
    };
    // #renderTextInput = (
    //     field: iField<string>,
    //     d: {
    //         id: string;
    //         title: NonNullable<ReactNode>;
    //         description?: NonNullable<ReactNode>;
    //         placeholder?: string;
    //     },
    // ): ReactElement => {
    //     const { hasUnsavedChanges } = this.props;
    //     const { value, disabled, onChange, error } = field;
    //     const $id = $(d.id);
    //
    //     return (
    //         <Field
    //             key={$id}
    //             title={d.title}
    //             description={d.description}
    //             disabled={disabled}
    //             className={cn(hasUnsavedChanges && css.unsavedField)}
    //         >
    //             <TextInput
    //                 id={$id}
    //                 labelText=""
    //                 hideLabel
    //                 value={value ?? ''}
    //                 onFocus={selfSelect}
    //                 placeholder={d.placeholder}
    //                 onChange={e => onChange(e.target.value)}
    //                 disabled={disabled}
    //                 invalid={!!error}
    //                 invalidText={error}
    //             />
    //         </Field>
    //     );
    // };

    //
    // Wifi
    //

    #wifiNetToString = (x: Maybe<pb.WifiNetwork>): string => {
        return pb.wifiNetworkToString(x, this.#txt.other) || 'N/A';
    };
    #wifiNetToElementLabel = (x: pb.WifiNetwork): ReactElement => {
        return <WifiNetworkLine net={x} variant="inline" />;
    };
    #wifiNetToElementMenu = (x: pb.WifiNetwork): ReactElement => {
        return (
            <WifiNetworkLine
                net={
                    x.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid
                        ? pb.create(pb.WifiNetworkSchema, {
                              ssid: this.#txt.other,
                          })
                        : x
                }
                variant="dropdown"
            />
        );
    };

    #wifiManualEntryDialogToggle = (desiredOpenState?: unknown | boolean): void => {
        let openDialog: State['openDialog'] = null;

        if (typeof desiredOpenState === 'boolean') openDialog = desiredOpenState ? 'wifiManualConnect' : null;
        else openDialog = this.state.openDialog === 'wifiManualConnect' ? null : 'wifiManualConnect';

        if (openDialog === this.state.openDialog) return;
        this.setState({ openDialog });
    };
    #wifiManualEntryDialogSubmit = async (): Promise<void> => {
        const {
            wifiActiveNetwork: { onConnectionRequest },
            intl: { formatMessage },
        } = this.props;
        const { openDialog, wifiManualConnect } = this.state;
        const { ssid, password, security } = wifiManualConnect.data;

        if (openDialog !== 'wifiManualConnect') return;

        const fail = <Key extends keyof ManualWifiApConfig>(key: Key, message: ManualWifiApConfig[Key]) => {
            this.setState(s => ({
                wifiManualConnect: {
                    ...s.wifiManualConnect,
                    [key]: message,
                },
            }));
        };
        if (!ssid) return fail('ssid', this.#txt.requiredField);
        if (!password) return fail('password', this.#txt.requiredField);

        // If we didn't bail till now, we can try to connect
        try {
            await setState(this, { wifiIsConnecting: true });
            await onConnectionRequest(ssid, security, password);
            await setState(this, getInitialState);
        } catch (e: any) {
            this.setState(s => ({
                wifiManualConnect: {
                    ...s.wifiManualConnect,
                    errors: { form: e.message || formatMessage({ defaultMessage: 'Unknown connection error' }) },
                },
            }));
        }
    };
    #wifiManualEntryDialogChange = <K extends keyof ManualWifiApConfig>(
        field: K,
        value: ManualWifiApConfig[K],
    ): void => {
        this.setState(s => ({
            wifiConnectionError: null,
            wifiManualConnect: {
                ...s.wifiManualConnect,
                errors: null,
                data: {
                    ...s.wifiManualConnect.data,
                    [field]: value,
                },
            },
        }));
    };
    #wifiManualEntryDialogRender = (): ReactElement => {
        const { intl } = this.props;
        const { formatMessage } = intl;

        const {
            openDialog,
            wifiIsConnecting,
            wifiManualConnect: { data, errors },
        } = this.state;

        const title = formatMessage({ defaultMessage: 'Find and join a Wi-Fi network' });
        const txt = this.#txt;

        return (
            <Modal
                id="boser-wifi-connect-custom"
                size="sm"
                selectorPrimaryFocus="input"
                open={openDialog === 'wifiManualConnect'}
                modalHeading={title}
                // Close
                onRequestClose={wifiIsConnecting ? undefined : this.#wifiManualEntryDialogToggle}
                onSecondarySubmit={wifiIsConnecting ? undefined : this.#wifiManualEntryDialogToggle}
                secondaryButtonText={wifiIsConnecting ? undefined : formatMessage({ defaultMessage: 'Cancel' })}
                // Submit
                onRequestSubmit={this.#wifiManualEntryDialogSubmit}
                primaryButtonDisabled={wifiIsConnecting}
                primaryButtonText={wifiIsConnecting ? txt.connectingElement : txt.join}
                className={css.wifiDialogManualEntry}
            >
                <Form className={css.wifiDialogForm}>
                    <div>
                        <TextInput
                            id="wifi-manual-ssid"
                            labelText={txt.ssid}
                            value={data.ssid}
                            disabled={wifiIsConnecting}
                            invalid={!!errors?.ssid}
                            invalidText={errors?.ssid}
                            onChange={e => {
                                this.#wifiManualEntryDialogChange('ssid', e.target.value);
                            }}
                            onFocus={selfSelect}
                        />
                    </div>

                    <div>
                        <Select
                            id="wifi-manual-security"
                            labelText={txt.security}
                            value={data.security}
                            disabled={wifiIsConnecting}
                            invalid={!!errors?.security}
                            invalidText={errors?.security}
                            children={pb.wifiEncryptionTypeOptions.map(x => {
                                return (
                                    <SelectItem
                                        key={x}
                                        value={x}
                                        text={pb.wifiEncryptionTypeToString(intl, x) ?? 'N/A'}
                                    />
                                );
                            })}
                            onChange={e => {
                                const value = Number.parseInt(e.target.value, 10) as pb.EncryptionType;
                                this.#wifiManualEntryDialogChange('security', value);
                            }}
                        />
                    </div>

                    {data.security === pb.EncryptionType.NONE ? null : (
                        <div>
                            <PasswordInput
                                id="wifi-manual-password"
                                lang="g'auld"
                                autoComplete="off"
                                disabled={wifiIsConnecting}
                                labelText={txt.password}
                                value={data.password}
                                invalid={!!errors?.password}
                                invalidText={errors?.password}
                                onChange={e => {
                                    this.#wifiManualEntryDialogChange('password', e.target.value);
                                }}
                                onFocus={selfSelect}
                            />
                        </div>
                    )}
                </Form>
            </Modal>
        );
    };

    #wifiPasswordDialogToggle = (ap: null | pb.WifiNetwork): void => {
        if (!ap) {
            this.props.wifiActiveNetwork.onConnectionRequestCancel?.();
            this.setState({
                wifiConnectionError: null,
                openDialog: null,
                wifiPasswordEntry: getInitialState().wifiPasswordEntry,
            });
            return;
        }

        this.setState({
            wifiConnectionError: null,
            openDialog: 'wifiPasswordEntry',
            wifiPasswordEntry: { wifiCell: ap, password: '', passwordError: null },
        });
    };
    #wifiPassowrdDialogSubmit = async (): Promise<void> => {
        const { onConnectionRequest } = this.props.wifiActiveNetwork;
        const { openDialog, wifiPasswordEntry } = this.state;
        const { password, wifiCell } = wifiPasswordEntry;

        const txt = this.#txt;

        if (!wifiCell || openDialog !== 'wifiPasswordEntry') return;
        if (!password) {
            this.setState(s => ({
                wifiPasswordEntry: {
                    ...s.wifiPasswordEntry,
                    passwordError: txt.passwordIsRequired,
                },
            }));
            return;
        }

        // If we didn't bail till now, we can try to connect
        try {
            await setState(this, { wifiIsConnecting: true });
            await onConnectionRequest(wifiCell.ssid, wifiCell.encryptionType, password);
            this.setState(getInitialState);
        } catch (e: any) {
            this.setState(s => ({
                wifiIsConnecting: false,
                wifiPasswordEntry: {
                    ...s.wifiPasswordEntry,
                    passwordError: e.message,
                },
            }));
        } finally {
            this.setState({ wifiIsConnecting: false });
        }
    };
    #wifiPasswordDialogRender = (): ReactElement => {
        const { intl, strings } = this.props;
        const {
            openDialog,
            wifiIsConnecting,
            wifiPasswordEntry: { wifiCell, password, passwordError },
        } = this.state;

        const txt = this.#txt;
        const title = intl.formatMessage(
            { defaultMessage: 'Password for {ssid}' },
            {
                ssid: <code children={wifiCell?.ssid || 'N/A'} />,
            },
        );
        const handleClose = () => this.#wifiPasswordDialogToggle(null);

        return (
            <Modal
                id="boser-wifi-password-entry"
                size="sm"
                selectorPrimaryFocus="input"
                open={openDialog === 'wifiPasswordEntry'}
                modalHeading={title}
                // Close
                onRequestClose={wifiIsConnecting ? undefined : handleClose}
                onSecondarySubmit={wifiIsConnecting ? undefined : handleClose}
                secondaryButtonText={wifiIsConnecting ? undefined : this.#txt.cancel}
                // Submit
                primaryButtonDisabled={wifiIsConnecting}
                onRequestSubmit={this.#wifiPassowrdDialogSubmit}
                primaryButtonText={wifiIsConnecting ? this.#txt.connectingElement : strings.wifiConnect}
                className={css.wifiDialogPassword}
            >
                <Form className={css.wifiDialogForm}>
                    <div>
                        <PasswordInput
                            id="wifi-password-entry"
                            lang="g'auld"
                            autoComplete="off"
                            labelText={txt.password}
                            value={password}
                            disabled={wifiIsConnecting}
                            invalid={!!passwordError}
                            invalidText={passwordError}
                            // The show password button tooltip overflows the dialog content and causes a vertical scrollbar.
                            // Moving it to the top causes the same problem, so we'll just move it to the left.
                            tooltipPosition="left"
                            onChange={e => {
                                this.setState(s => ({
                                    wifiPasswordEntry: {
                                        ...s.wifiPasswordEntry,
                                        password: e.target.value,
                                        passwordError: null,
                                    },
                                }));
                            }}
                        />
                    </div>
                </Form>
            </Modal>
        );
    };

    #handleWifiNetworkChange = async (ap: null | pb.WifiNetwork): Promise<void> => {
        const { onConnectionRequest } = this.props.wifiActiveNetwork;
        const { wifiManualConnect, wifiPasswordEntry } = getInitialState();
        await setState(this, { wifiConnectionError: null, wifiManualConnect, wifiPasswordEntry });

        // Nothing to do when the selection is somehow cleared
        if (ap == null) {
            return;
        }

        // Manual entry dialog
        else if (ap.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid) {
            // Reset the dropdown state to make de-select the "Other…" option
            this.setState(s => ({ ...s, wifiNetworkDropdownKey: s.wifiNetworkDropdownKey + 1 }));
            this.#wifiManualEntryDialogToggle(true);
        }

        // Propagate the change upstream
        // and try to connect to the network
        else {
            this.props.wifiActiveNetwork.onChange(ap);

            // If the network is open, we can connect right away
            if (ap.encryptionType === pb.EncryptionType.NONE) {
                onConnectionRequest(ap.ssid, ap.encryptionType, '').catch((e: any) => {
                    this.setState({ wifiConnectionError: e.message });
                });
            }

            // …otherwise we gotta go throught the password dialog
            else this.#wifiPasswordDialogToggle(ap);
        }
    };
    #renderWifi = (): ReactNode => {
        const { wifiAvailableNetworks, wifiActiveNetwork } = this.props;
        const { wifiNetworkDropdownKey } = this.state;

        // const globalManualConnectError = this.state.wifiManualConnect.errors?.form;
        const txt = this.#txt;

        return (
            <div className={css.wifiDropdownWrapper} data-floating-menu-container>
                <Dropdown<pb.WifiNetwork>
                    // downshiftProps={{ isOpen: true }}
                    id="wifi-network"
                    key={[wifiNetworkDropdownKey, wifiActiveNetwork.value?.ssid ?? ''].join('-')}
                    type="default"
                    direction="bottom"
                    // Labeling / visuals
                    titleText={txt.network}
                    label={<span children={txt.selectWifiNetwork} className={css.label} />}
                    hideLabel
                    // Behavior & appearance
                    size="md"
                    // The positioning gets tripped up
                    // with autoAlign algo active
                    autoAlign={false}
                    disabled={wifiAvailableNetworks.isLoading}
                    invalid={!!wifiActiveNetwork.error}
                    invalidText={wifiActiveNetwork.error}
                    helperText={wifiAvailableNetworks.isLoading ? txt.scanningElement : null}
                    // Value
                    items={[...wifiAvailableNetworks.options, WIFI_AP_OTHER_PLACEHOLDER]}
                    selectedItem={wifiActiveNetwork.value ?? undefined}
                    onChange={x => this.#handleWifiNetworkChange(x.selectedItem)}
                    // Item rendering
                    itemToString={this.#wifiNetToString}
                    itemToElement={this.#wifiNetToElementMenu}
                    renderSelectedItem={this.#wifiNetToElementLabel}
                />
                <div
                    // Button with tooltip has annoying DOM structure,
                    // so we need to wrap it in a div that will give
                    // the whole thing the right positioning
                    className={css.wifiNetSelectorRefreshButton}
                >
                    <Button
                        kind="secondary"
                        size="md"
                        hasIconOnly
                        renderIcon={IconRefresh}
                        title={this.#txt.refresh}
                        onClick={wifiAvailableNetworks.onRefresh}
                        disabled={wifiAvailableNetworks.isLoading}
                        loading={wifiAvailableNetworks.isLoading}
                    />
                </div>

                {this.#wifiManualEntryDialogRender()}
                {this.#wifiPasswordDialogRender()}
            </div>
        );
    };

    render() {
        const {
            intl: { formatMessage },

            // Fields
            hostname,
            // protocol,
            // staticAddress,
            // staticNetmask,
            // staticGateway,
            // staticDns,

            // Form
            // hasUnsavedChanges,
            // onSave,
            // onReset,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={null}>
                    <Field title={formatMessage({ defaultMessage: 'Status' })} children={this.#renderStats()} />

                    {hostname != null ? (
                        <Field title={formatMessage({ defaultMessage: 'Hostname' })} disabled={hostname.disabled}>
                            <TextInput
                                id={$('hostname')}
                                type="text"
                                labelText=""
                                hideLabel
                                disabled={hostname.disabled}
                                value={hostname.value ?? ''}
                                onChange={e => hostname.onChange(e.target.value)}
                                invalid={!!hostname.error}
                                invalidText={hostname.error}
                            />
                        </Field>
                    ) : null}

                    <Field title={formatMessage({ defaultMessage: 'Network' })} children={this.#renderWifi()} />

                    {/*
                    <Field
                        className={cn(hasUnsavedChanges && css.unsavedField)}
                        title={formatMessage({ defaultMessage: 'Protocol' })}
                        disabled={protocol.disabled}
                    >
                        <ButtonSwitch<NetProto>
                            selectedOption={protocol.value}
                            options={[
                                { id: 'dhcp', text: formatMessage({ defaultMessage: 'DHCP' }) },
                                { id: 'static', text: formatMessage({ defaultMessage: 'Static' }) },
                            ]}
                            size="md"
                            disabled={protocol.disabled}
                            onChange={protocol.onChange}
                            invalid={!!protocol.error}
                            invalidText={protocol.error}
                        />
                    </Field>

                    {protocol.value === 'static'
                        ? [
                              this.#renderTextInput(staticAddress, {
                                  id: 'address',
                                  placeholder: '0.0.0.0',
                                  title: formatMessage({ defaultMessage: 'IP Address' }),
                              }),
                              this.#renderTextInput(staticNetmask, {
                                  id: 'netmask',
                                  placeholder: '0.0.0.0',
                                  title: formatMessage({ defaultMessage: 'Netmask' }),
                              }),
                              this.#renderTextInput(staticGateway, {
                                  id: 'gateway',
                                  placeholder: '0.0.0.0',
                                  title: formatMessage({ defaultMessage: 'Gateway' }),
                              }),
                              this.#renderTextInput(staticDns, {
                                  id: 'dns',
                                  placeholder: '0.0.0.0, 1.1.1.1, 3.3.3.3',
                                  title: formatMessage({ defaultMessage: 'DNS Servers' }),
                                  description: formatMessage({ defaultMessage: 'Comma separated values' }),
                              }),
                          ]
                        : null}

                    {hasUnsavedChanges ? (
                        <footer className={cn(css.footer, hasUnsavedChanges && css.unsavedField)}>
                            <Button kind="primary" children="Save Changes" onClick={onSave} />
                            <Button kind="secondary" children="Reset" onClick={onReset} />
                        </footer>
                    ) : null}
                    */}
                </FieldSet>
            </Form>
        );
    }
}

export function SectionSettings(props: SectionSettingsProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
