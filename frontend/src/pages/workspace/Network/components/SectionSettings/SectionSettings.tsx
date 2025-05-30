import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

// Lib
// import { assertUnreachable } from '@/lib/ts';
import { AutoSelected, selfSelect /*, setState*/ } from '@/lib/react';
import { Form, type iField, getID } from '@/lib/form';

// App
import type * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import {
    Field,
    FieldSet,
    ButtonSwitch,
    Button,
    // Modal,
    // Wifi as IconWifi,
} from '@/components';
// import { Locked as IconLocked, Unlocked as IconUnlocked, Renew as IconRefresh } from '@carbon/react/icons';
import {
    TextInput,
    // InlineLoading,
    // Dropdown,
    // Select,
    // SelectItem,
    // PasswordInput,
} from '@carbon/react';

// Styles
import cn from 'clsx';
import css from './SectionSettings.scss';

type NetProto = NonNullable<pb.NetworkConfig['protocol']['case']>;

export interface SectionSettingsProps {
    status: Array<[label: ReactNode, value: ReactNode]>;

    hostname: null | iField<string>;
    protocol: iField<NetProto>;
    staticAddress: iField<string>;
    staticNetmask: iField<string>;
    staticGateway: iField<string>;
    staticDns: iField<string>;

    // Wifi
    // The connected bool attribute is remapped to nullability
    // strings: { wifiConnect: string };
    // wifiActiveNetwork: iField<Maybe<WifiCell>> & {
    //     onConnectionRequest(ssid: string, security: EncryptionType, password: string): Promise<boolean>;
    //     onConnectionRequestCancel?(): void;
    // };
    // wifiAvailableNetworks: {
    //     isLoading: boolean;
    //     options: WifiCell[];
    //     onRefresh(): void;
    // };

    hasUnsavedChanges: boolean;
    onSave(): void;
    onReset(): void;
}
interface Props extends SectionSettingsProps {
    intl: IntlShape;
}

// export enum EncryptionType {
//     None = 'NONE',
//     Wep = 'WEP',
//     WepShared = 'WEP_SHARED',
//     Wpa = 'WPA',
//     Wpa_1_2 = 'WPA_1_2',
//     Wpa_2 = 'WPA_2',
//     Wpa_2_3 = 'WPA_2_3',
// }
// export enum SignalStrength {
//     Excellent = 'EXCELLENT',
//     Fair = 'FAIR',
//     Low = 'LOW',
//     Offline = 'OFFLINE',
// }
// export type WifiCell = {
//     connected: boolean;
//     encryptionType: EncryptionType;
//     signalStrength: SignalStrength;
//     ssid: string;
// };
//
// interface WifiApEncryptionInfo {
//     key: EncryptionType;
//     label: string;
//     security: 'none' | 'low' | 'fine';
//     icon: ReactElement;
// }
// interface WifiApToElementProps extends WifiCell {
//     lockIconPosition?: 'start' | 'end';
//     className?: string;
//     other: string;
// }
//
// /**
//  * The selection dropdown is a Downshift instance,
//  * which requires all options to be of the same type.
//  *
//  * This is a placeholder object that represents the "Other…" option.
//  * It needs to be caught and handled in special way in…
//  *  - renderToString
//  *  - renderToElement
//  *  - onChange
//  */
// export const WIFI_AP_OTHER_PLACEHOLDER: Readonly<WifiCell> = Object.freeze({
//     ssid: 'WIFI_AP_OTHER_PLACEHOLDER',
//     macAddress: 'WIFI_AP_OTHER_PLACEHOLDER',
//     connected: false,
//     encryptionType: EncryptionType.None,
//     signalStrength: SignalStrength.Offline,
// });

// interface ManualWifiApConfig {
//     ssid: string;
//     security: EncryptionType;
//     password: string;
// }
interface State {
    // openDialog: null | 'wifiManualConnect' | 'wifiPasswordEntry';
    //
    // wifiManualConnect: {
    //     data: ManualWifiApConfig;
    //     errors: null | Partial<Record<keyof ManualWifiApConfig | 'form', string>>;
    // };
    // wifiPasswordEntry: {
    //     wifiCell: null | WifiCell;
    //     password: string;
    //     passwordError: null | string;
    // };
    // wifiConnectionError: null | string;
    // wifiNetworkDropdownKey: number;
    // wifiIsConnecting: boolean;
}
const getInitialState = (): State => ({
    // openDialog: null,
    //
    // wifiManualConnect: {
    //     data: {
    //         ssid: '',
    //         security: EncryptionType.Wpa_2_3,
    //         password: '',
    //     },
    //     errors: null,
    // },
    // wifiPasswordEntry: {
    //     wifiCell: null,
    //     password: '',
    //     passwordError: null,
    // },
    // wifiConnectionError: null,
    // wifiNetworkDropdownKey: 0,
    // wifiIsConnecting: false,
});

class View extends Component<Props, State> {
    readonly state = getInitialState();
    static contextType = AppContext;
    declare context: AppContextType;

    #id = getID('settings', 'network', 'config');
    // get #txt() {
    //     const { formatMessage } = this.props.intl;
    //     return {
    //         security: formatMessage({ defaultMessage: 'Security' }),
    //         network: formatMessage({ defaultMessage: 'Network' }),
    //         password: formatMessage({ defaultMessage: 'Password' }),
    //
    //         // Action words
    //         refresh: formatMessage({ defaultMessage: 'Refresh' }),
    //         cancel: formatMessage({ defaultMessage: 'Cancel' }),
    //         join: formatMessage({ defaultMessage: 'Join' }),
    //
    //         // Validation
    //         requiredField: formatMessage({ defaultMessage: 'This field is required' }),
    //         passwordIsRequired: formatMessage({ defaultMessage: 'Password is required' }),
    //
    //         // Wifi
    //         wifi: formatMessage({ defaultMessage: 'Wi-Fi' }),
    //         ssid: formatMessage({ defaultMessage: 'SSID' }),
    //         other: formatMessage({ defaultMessage: 'Other…' }),
    //         selectWifiNetwork: formatMessage({ defaultMessage: 'Select a Wi-Fi Network…' }),
    //         connecting: (
    //             <InlineLoading status="active" description={formatMessage({ defaultMessage: 'Connecting…' })} />
    //         ),
    //     };
    // }

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
    #renderTextInput = (
        field: iField<string>,
        d: {
            id: string;
            title: NonNullable<ReactNode>;
            description?: NonNullable<ReactNode>;
            placeholder?: string;
        },
    ): ReactElement => {
        const { hasUnsavedChanges } = this.props;
        const { value, disabled, onChange, error } = field;
        const $id = this.#id.get(d.id);

        return (
            <Field
                key={$id}
                title={d.title}
                description={d.description}
                disabled={disabled}
                className={cn(hasUnsavedChanges && css.unsavedField)}
            >
                <TextInput
                    id={$id}
                    labelText=""
                    hideLabel
                    value={value ?? ''}
                    onFocus={selfSelect}
                    placeholder={d.placeholder}
                    onChange={e => onChange(e.target.value)}
                    disabled={disabled}
                    invalid={!!error}
                    invalidText={error}
                />
            </Field>
        );
    };

    //
    // Wifi
    //

    // #getWifiApEncryptionInfo = (encryptionType: EncryptionType): WifiApEncryptionInfo => {
    //     const { formatMessage } = this.props.intl;
    //
    //     switch (encryptionType) {
    //         case EncryptionType.None:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconUnlocked />,
    //                 label: formatMessage({ defaultMessage: 'None' }),
    //                 security: 'none',
    //             };
    //
    //         case EncryptionType.Wep:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WEP' }),
    //                 security: 'low',
    //             };
    //         case EncryptionType.WepShared:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WEP Shared' }),
    //                 security: 'low',
    //             };
    //
    //         case EncryptionType.Wpa:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WPA' }),
    //                 security: 'fine',
    //             };
    //         case EncryptionType.Wpa_1_2:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WPA 1/2' }),
    //                 security: 'fine',
    //             };
    //         case EncryptionType.Wpa_2:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WPA 2' }),
    //                 security: 'fine',
    //             };
    //         case EncryptionType.Wpa_2_3:
    //             return {
    //                 key: encryptionType,
    //                 icon: <IconLocked />,
    //                 label: formatMessage({ defaultMessage: 'WPA 2/3' }),
    //                 security: 'fine',
    //             };
    //         // case EncryptionType.Wpa_3:
    //         //     return {
    //         //         key: encryptionType,
    //         //         icon: <IconLocked />,
    //         //         label: formatMessage({ defaultMessage: 'WPA 3' }),
    //         //         security: 'fine',
    //         //     };
    //
    //         default:
    //             assertUnreachable(encryptionType);
    //     }
    // };
    // #encryptionOptions: Array<WifiApEncryptionInfo> = [
    //     EncryptionType.None,
    //     EncryptionType.Wep,
    //     EncryptionType.WepShared,
    //     EncryptionType.Wpa,
    //     EncryptionType.Wpa_1_2,
    //     EncryptionType.Wpa_2,
    //     EncryptionType.Wpa_2_3,
    // ].map(this.#getWifiApEncryptionInfo);

    // #wifiApToString = (wifiAp: null | WifiCell): string => {
    //     if (wifiAp == null) return '';
    //     return wifiAp.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid ? this.#txt.other : wifiAp.ssid;
    // };
    // #wifiApToElement = (props: WifiApToElementProps): ReactElement => {
    //     const { ssid, signalStrength, encryptionType, className, lockIconPosition, other } = props;
    //
    //     const iconStrength = (
    //         {
    //             [SignalStrength.Offline]: 'offline',
    //             [SignalStrength.Low]: 'low',
    //             [SignalStrength.Fair]: 'fair',
    //             [SignalStrength.Excellent]: 'full',
    //         } as const
    //     )[signalStrength];
    //     const secInfo = this.#getWifiApEncryptionInfo(encryptionType);
    //
    //     const isOtherPlaceholder: boolean = ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid;
    //     const label: string = isOtherPlaceholder ? other : ssid;
    //     const title: undefined | string = isOtherPlaceholder ? undefined : ssid;
    //
    //     let $lockIcon: ReactNode;
    //     if (!isOtherPlaceholder) {
    //         $lockIcon = (
    //             <span className={css.encryption} title={secInfo.label}>
    //                 <span children={secInfo.icon} className={css[secInfo.security]} />
    //             </span>
    //         );
    //     }
    //
    //     return (
    //         <div className={cn(css.wifiAp, className)}>
    //             <span className={css.left}>
    //                 {isOtherPlaceholder ? null : <IconWifi size={16} state={iconStrength} />}
    //                 {lockIconPosition === 'start' && $lockIcon}
    //                 <span className={css.ssid} children={label} title={title} />
    //             </span>
    //
    //             {lockIconPosition === 'end' && $lockIcon}
    //         </div>
    //     );
    // };
    // #wifiCellToElementIconStart = (v: WifiCell): ReactElement => {
    //     return this.#wifiApToElement({ ...v, other: this.#txt.other, lockIconPosition: 'start' });
    // };

    // #handleWifiManualEntryDialogToggle = (desiredOpenState?: unknown | boolean): void => {
    //     let openDialog: State['openDialog'] = null;
    //
    //     if (typeof desiredOpenState === 'boolean') openDialog = desiredOpenState ? 'wifiManualConnect' : null;
    //     else openDialog = this.state.openDialog === 'wifiManualConnect' ? null : 'wifiManualConnect';
    //
    //     if (openDialog === this.state.openDialog) return;
    //     this.setState({ openDialog });
    // };
    // #handleWifiManualEntryDialogSubmit = async (): Promise<void> => {
    //     const {
    //         wifiActiveNetwork: { onConnectionRequest },
    //         intl: { formatMessage },
    //     } = this.props;
    //     const { openDialog, wifiManualConnect } = this.state;
    //     const { ssid, password, security } = wifiManualConnect.data;
    //
    //     if (openDialog !== 'wifiManualConnect') return;
    //
    //     const fail = <Key extends keyof ManualWifiApConfig>(key: Key, message: ManualWifiApConfig[Key]) => {
    //         this.setState(s => ({
    //             wifiManualConnect: {
    //                 ...s.wifiManualConnect,
    //                 [key]: message,
    //             },
    //         }));
    //     };
    //     if (!ssid) return fail('ssid', this.#txt.requiredField);
    //     if (!password) return fail('password', this.#txt.requiredField);
    //
    //     // If we didn't bail till now, we can try to connect
    //     try {
    //         await setState(this, { wifiIsConnecting: true });
    //         await onConnectionRequest(ssid, security, password);
    //         await setState(this, getInitialState);
    //     } catch (e: any) {
    //         this.setState(s => ({
    //             wifiManualConnect: {
    //                 ...s.wifiManualConnect,
    //                 errors: { form: e.message || formatMessage({ defaultMessage: 'Unknown connection error' }) },
    //             },
    //         }));
    //     }
    // };
    // #handleWifiManualEntryDialogChange = <K extends keyof ManualWifiApConfig>(
    //     field: K,
    //     value: ManualWifiApConfig[K],
    // ): void => {
    //     this.setState(s => ({
    //         wifiConnectionError: null,
    //         wifiManualConnect: {
    //             ...s.wifiManualConnect,
    //             errors: null,
    //             data: {
    //                 ...s.wifiManualConnect.data,
    //                 [field]: value,
    //             },
    //         },
    //     }));
    // };
    // #renderWifiDialogManualEntry = (): ReactElement => {
    //     const { formatMessage } = this.props.intl;
    //     const {
    //         openDialog,
    //         wifiIsConnecting,
    //         wifiManualConnect: { data, errors },
    //     } = this.state;
    //     const title = formatMessage({ defaultMessage: 'Find and join a Wi-Fi network' });
    //     const txt = this.#txt;
    //
    //     return (
    //         <Modal
    //             id="boser-wifi-connect-custom"
    //             size="sm"
    //             selectorPrimaryFocus="input"
    //             open={openDialog === 'wifiManualConnect'}
    //             modalHeading={title}
    //             // Close
    //             onRequestClose={wifiIsConnecting ? undefined : this.#handleWifiManualEntryDialogToggle}
    //             onSecondarySubmit={wifiIsConnecting ? undefined : this.#handleWifiManualEntryDialogToggle}
    //             secondaryButtonText={wifiIsConnecting ? undefined : formatMessage({ defaultMessage: 'Cancel' })}
    //             // Submit
    //             onRequestSubmit={this.#handleWifiManualEntryDialogSubmit}
    //             primaryButtonDisabled={wifiIsConnecting}
    //             primaryButtonText={wifiIsConnecting ? txt.connecting : txt.join}
    //             className={css.wifiDialogManualEntry}
    //         >
    //             <Form className={css.wifiDialogForm}>
    //                 <div>
    //                     <TextInput
    //                         id="wifi-manual-ssid"
    //                         labelText={txt.ssid}
    //                         value={data.ssid}
    //                         disabled={wifiIsConnecting}
    //                         invalid={!!errors?.ssid}
    //                         invalidText={errors?.ssid}
    //                         onChange={e => {
    //                             this.#handleWifiManualEntryDialogChange('ssid', e.target.value);
    //                         }}
    //                         onFocus={selfSelect}
    //                     />
    //                 </div>
    //
    //                 <div>
    //                     <Select
    //                         id="wifi-manual-security"
    //                         labelText={txt.security}
    //                         value={data.security}
    //                         disabled={wifiIsConnecting}
    //                         invalid={!!errors?.security}
    //                         invalidText={errors?.security}
    //                         children={this.#encryptionOptions.map(({ key, label }) => {
    //                             return <SelectItem key={key} value={key} text={label} />;
    //                         })}
    //                         onChange={e => {
    //                             this.#handleWifiManualEntryDialogChange('security', e.target.value as EncryptionType);
    //                         }}
    //                     />
    //                 </div>
    //
    //                 {data.security === EncryptionType.None ? null : (
    //                     <div>
    //                         <PasswordInput
    //                             id="wifi-manual-password"
    //                             lang="g'auld"
    //                             autoComplete="off"
    //                             disabled={wifiIsConnecting}
    //                             labelText={txt.password}
    //                             value={data.password}
    //                             invalid={!!errors?.password}
    //                             invalidText={errors?.password}
    //                             onChange={e => {
    //                                 this.#handleWifiManualEntryDialogChange('password', e.target.value);
    //                             }}
    //                             onFocus={selfSelect}
    //                         />
    //                     </div>
    //                 )}
    //             </Form>
    //         </Modal>
    //     );
    // };

    // #handleWifiPasswordToggle = (ap: null | WifiCell): void => {
    //     if (!ap) {
    //         this.props.wifiActiveNetwork.onConnectionRequestCancel?.();
    //         this.setState({
    //             wifiConnectionError: null,
    //             openDialog: null,
    //             wifiPasswordEntry: getInitialState().wifiPasswordEntry,
    //         });
    //         return;
    //     }
    //
    //     this.setState({
    //         wifiConnectionError: null,
    //         openDialog: 'wifiPasswordEntry',
    //         wifiPasswordEntry: { wifiCell: ap, password: '', passwordError: null },
    //     });
    // };
    // #handleWifiPasswordSubmit = async (): Promise<void> => {
    //     const { onConnectionRequest } = this.props.wifiActiveNetwork;
    //     const { openDialog, wifiPasswordEntry } = this.state;
    //     const { password, wifiCell } = wifiPasswordEntry;
    //
    //     const txt = this.#txt;
    //
    //     if (!wifiCell || openDialog !== 'wifiPasswordEntry') return;
    //     if (!password) {
    //         this.setState(s => ({
    //             wifiPasswordEntry: {
    //                 ...s.wifiPasswordEntry,
    //                 passwordError: txt.passwordIsRequired,
    //             },
    //         }));
    //         return;
    //     }
    //
    //     // If we didn't bail till now, we can try to connect
    //     try {
    //         await setState(this, { wifiIsConnecting: true });
    //         await onConnectionRequest(wifiCell.ssid, wifiCell.encryptionType, password);
    //         this.setState(getInitialState);
    //     } catch (e: any) {
    //         this.setState(s => ({
    //             wifiIsConnecting: false,
    //             wifiPasswordEntry: {
    //                 ...s.wifiPasswordEntry,
    //                 passwordError: e.message,
    //             },
    //         }));
    //     } finally {
    //         this.setState({ wifiIsConnecting: false });
    //     }
    // };
    // #renderWifiDialogPassword = (): ReactElement => {
    //     const { intl, strings } = this.props;
    //     const {
    //         openDialog,
    //         wifiIsConnecting,
    //         wifiPasswordEntry: { wifiCell, password, passwordError },
    //     } = this.state;
    //
    //     const txt = this.#txt;
    //     const title = intl.formatMessage(
    //         { defaultMessage: 'Password for {ssid}' },
    //         {
    //             ssid: <code children={wifiCell?.ssid || 'N/A'} />,
    //         },
    //     );
    //     const handleClose = () => this.#handleWifiPasswordToggle(null);
    //
    //     return (
    //         <Modal
    //             id="boser-wifi-password-entry"
    //             size="sm"
    //             selectorPrimaryFocus="input"
    //             open={openDialog === 'wifiPasswordEntry'}
    //             modalHeading={title}
    //             // Close
    //             onRequestClose={wifiIsConnecting ? undefined : handleClose}
    //             onSecondarySubmit={wifiIsConnecting ? undefined : handleClose}
    //             secondaryButtonText={wifiIsConnecting ? undefined : this.#txt.cancel}
    //             // Submit
    //             primaryButtonDisabled={wifiIsConnecting}
    //             onRequestSubmit={this.#handleWifiPasswordSubmit}
    //             primaryButtonText={wifiIsConnecting ? this.#txt.connecting : strings.wifiConnect}
    //             className={css.wifiDialogPassword}
    //         >
    //             <Form className={css.wifiDialogForm}>
    //                 <div>
    //                     <PasswordInput
    //                         id="wifi-password-entry"
    //                         lang="g'auld"
    //                         autoComplete="off"
    //                         labelText={txt.password}
    //                         value={password}
    //                         disabled={wifiIsConnecting}
    //                         invalid={!!passwordError}
    //                         invalidText={passwordError}
    //                         // The show password button tooltip overflows the dialog content and causes a vertical scrollbar.
    //                         // Moving it to the top causes the same problem, so we'll just move it to the left.
    //                         tooltipPosition="left"
    //                         onChange={e => {
    //                             this.setState(s => ({
    //                                 wifiPasswordEntry: {
    //                                     ...s.wifiPasswordEntry,
    //                                     password: e.target.value,
    //                                     passwordError: null,
    //                                 },
    //                             }));
    //                         }}
    //                     />
    //                 </div>
    //             </Form>
    //         </Modal>
    //     );
    // };

    // #handleWifiNetworkChange = async (ap: null | WifiCell): Promise<void> => {
    //     const { onConnectionRequest } = this.props.wifiActiveNetwork;
    //     const { wifiManualConnect, wifiPasswordEntry } = getInitialState();
    //     await setState(this, { wifiConnectionError: null, wifiManualConnect, wifiPasswordEntry });
    //
    //     // Nothing to do when the selection is somehow cleared
    //     if (ap == null) {
    //         return;
    //     }
    //
    //     // Manual entry dialog
    //     else if (ap.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid) {
    //         // Reset the dropdown state to make de-select the "Other…" option
    //         this.setState(s => ({ ...s, wifiNetworkDropdownKey: s.wifiNetworkDropdownKey + 1 }));
    //         this.#handleWifiManualEntryDialogToggle(true);
    //     }
    //
    //     // Propagate the change upstream
    //     // and try to connect to the network
    //     else {
    //         this.props.wifiActiveNetwork.onChange(ap);
    //
    //         // If the network is open, we can connect right away
    //         if (ap.encryptionType === EncryptionType.None) {
    //             onConnectionRequest(ap.ssid, ap.encryptionType, '').catch(e => {
    //                 this.setState({ wifiConnectionError: e.message });
    //             });
    //         }
    //
    //         // …otherwise we gotta go throught the password dialog
    //         else this.#handleWifiPasswordToggle(ap);
    //     }
    // };
    // #renderWifi = (): ReactNode => {
    //     const { wifiAvailableNetworks, wifiActiveNetwork } = this.props;
    //     const { wifiNetworkDropdownKey } = this.state;
    //
    //     // const globalManualConnectError = this.state.wifiManualConnect.errors?.form;
    //     const txt = this.#txt;
    //
    //     return (
    //         <div className={css.wifiNetSelectorWrapper} data-floating-menu-container>
    //             <Dropdown<WifiCell>
    //                 // downshiftProps={{ isOpen: true }}
    //                 id="wifi-network"
    //                 key={[wifiNetworkDropdownKey, wifiActiveNetwork.value?.ssid ?? ''].join('-')}
    //                 type="default"
    //                 direction="bottom"
    //                 // Labeling / visuals
    //                 className={css.wifiNetSelector}
    //                 titleText={txt.network}
    //                 label={txt.selectWifiNetwork}
    //                 hideLabel
    //                 // Behavior & appearance
    //                 size="md"
    //                 // The positioning gets tripped up
    //                 // with autoAlign algo active
    //                 autoAlign={false}
    //                 disabled={wifiAvailableNetworks.isLoading}
    //                 invalid={!!wifiActiveNetwork.error}
    //                 invalidText={wifiActiveNetwork.error}
    //                 // Value
    //                 items={[...wifiAvailableNetworks.options, WIFI_AP_OTHER_PLACEHOLDER]}
    //                 selectedItem={wifiActiveNetwork.value ?? undefined}
    //                 onChange={x => this.#handleWifiNetworkChange(x.selectedItem)}
    //                 // Item rendering
    //                 itemToString={v => this.#wifiApToString(v)}
    //                 itemToElement={this.#wifiCellToElementIconStart}
    //                 renderSelectedItem={this.#wifiCellToElementIconStart}
    //             />
    //             <div
    //                 // Button with tooltip has annoying DOM structure,
    //                 // so we need to wrap it in a div that will give
    //                 // the whole thing the right positioning
    //                 className={css.wifiNetSelectorRefreshButton}
    //             >
    //                 <Button
    //                     kind="secondary"
    //                     size="md"
    //                     hasIconOnly
    //                     renderIcon={IconRefresh}
    //                     title={this.#txt.refresh}
    //                     onClick={wifiAvailableNetworks.onRefresh}
    //                     disabled={wifiAvailableNetworks.isLoading}
    //                     loading={wifiAvailableNetworks.isLoading}
    //                 />
    //             </div>
    //
    //             {this.#renderWifiDialogManualEntry()}
    //             {this.#renderWifiDialogPassword()}
    //         </div>
    //     );
    // };

    render() {
        const {
            intl: { formatMessage },

            // Fields
            hostname,
            protocol,
            staticAddress,
            staticNetmask,
            staticGateway,
            staticDns,

            // Form
            hasUnsavedChanges,
            onSave,
            onReset,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={null}>
                    <Field title={formatMessage({ defaultMessage: 'Status' })} children={this.#renderStats()} />

                    {hostname != null ? (
                        <Field title={formatMessage({ defaultMessage: 'Hostname' })} disabled={hostname.disabled}>
                            <TextInput
                                id={this.#id.get('hostname')}
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
                </FieldSet>

                {/*
                <FieldSet title={formatMessage({ defaultMessage: 'WiFi' })}>
                    <Field title={formatMessage({ defaultMessage: 'Network' })} children={this.#renderWifi()} />
                </FieldSet>
                */}
            </Form>
        );
    }
}

export function SectionSettings(props: SectionSettingsProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
