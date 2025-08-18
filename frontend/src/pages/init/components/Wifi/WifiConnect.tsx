import { Component, Fragment } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

import { Form, getID } from '@/lib/form';
import * as pb from '@/proto';

// Components
import { Layout } from '../Layout';
import { DoneScene } from './DoneScene';
import { Renew as IconReload } from '@carbon/react/icons';
import { Button, LogoHeader, WifiNetworkLine } from '@/components';
import { Dropdown, Link, ProgressIndicator, ProgressStep, TextInput } from '@carbon/react';

// Styles
import css from './Wifi.scss';

export interface WifiConnectProps {
    isLoading: boolean;
    networks: pb.WifiNetwork[];
    onSelect(net: pb.WifiNetwork): void;
    onReload(): void;

    onBack(): void;
    onSubmit(net: pb.SetWifiRequest): Promise<boolean>;
}
interface Props extends WifiConnectProps {
    intl: IntlShape;
}

type Security = Exclude<pb.EncryptionType, pb.EncryptionType.UNSPECIFIED>;

interface State {
    isDone: boolean;
    isManualEntryActive: boolean;
    selectedNetwork: null | (pb.WifiNetwork & { password?: string });
    manualEntryData: pb.SetWifiRequest;
}
const getInitialState = (): State => ({
    isDone: false,
    isManualEntryActive: false,
    selectedNetwork: null,
    manualEntryData: pb.create(pb.SetWifiRequestSchema),
});

const $ = getID('initial-setup-wifi').get;
class View extends Component<Props, State> {
    readonly state = getInitialState();

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            ssidNetName: formatMessage({ defaultMessage: 'SSID (network name)' }),
            encryptionType: formatMessage({ defaultMessage: 'Encryption Type' }),
            password: formatMessage({ defaultMessage: 'Password' }),

            back: formatMessage({ defaultMessage: 'Back' }),
            confirm: formatMessage({ defaultMessage: 'Confirm' }),
        };
    }
    #renderEncryptionType = (type: Maybe<pb.EncryptionType>): string => {
        if (!type) return 'N/A';
        return pb.wifiEncryptionTypeToString(this.props.intl, type) || 'N/A';
    };
    #netloadingEntries = Array.from({ length: 5 }, (_, i) => <WifiNetworkLine.Skeleton variant="inline" key={i} />);

    //
    // Manual network entry
    //

    #manualEntryActivate = (): void => this.setState({ isManualEntryActive: true });
    #manualEntryDeactivate = (): void => this.setState({ isManualEntryActive: false });
    #manualValChange = <Key extends keyof pb.SetWifiRequest>(key: Key, value: pb.SetWifiRequest[Key]): void => {
        this.setState(s => ({
            manualEntryData: {
                ...s.manualEntryData,
                [key]: value,
            },
        }));
    };
    #manualEncryptionSelect = (x: { selectedItem: null | Security }): void => {
        this.#manualValChange('encryptionType', x.selectedItem ?? pb.EncryptionType.NONE);
    };

    //
    // Selection screen
    //

    #netSelect = (net: null | pb.WifiNetwork): void => {
        this.setState({ selectedNetwork: net });
    };
    #netDeselect = (): void => this.setState({ selectedNetwork: null });
    #netSetPass = (value: string) => {
        this.setState(s => ({
            selectedNetwork:
                s.selectedNetwork == null
                    ? null
                    : {
                          ...s.selectedNetwork,
                          password: value,
                      },
        }));
    };
    #netToString = (x: Maybe<pb.WifiNetwork>): string => {
        return pb.wifiNetworkToString(x, '') || 'N/A';
    };

    #netToElementLabel = (x: pb.WifiNetwork): ReactElement => {
        return <WifiNetworkLine net={x} variant="inline" />;
    };
    #netToElementMenu = (x: pb.WifiNetwork): ReactElement => {
        return <WifiNetworkLine net={x} variant="dropdown" />;
    };

    //
    // Control flow methods
    //

    #goBack = (): void => this.props.onBack();
    #submit = async (data: pb.SetWifiRequest): Promise<void> => {
        const res = await this.props.onSubmit(data);
        if (res) this.setState({ isDone: true });
    };

    render() {
        if (this.state.isDone) return <DoneScene />;

        const { networks, onReload, isLoading } = this.props;
        const { isManualEntryActive, selectedNetwork, manualEntryData } = this.state;
        const txt = this.#txt;

        let content: ReactNode = null;
        interface ControlButtonConf {
            label: string;
            onClick(): void;
            disabled?: boolean;
        }

        let goBackBtn: null | ControlButtonConf;
        let goNextBtn: null | ControlButtonConf;

        // Enter network details manually
        if (isManualEntryActive) {
            goBackBtn = { label: txt.back, onClick: this.#manualEntryDeactivate };
            goNextBtn = { label: txt.confirm, onClick: () => this.#submit(this.state.manualEntryData) };

            content = (
                <Fragment>
                    <h1 className={css.title} children="Enter Wifi details manually" />

                    <Form className={css.manualForm}>
                        <TextInput
                            id={$('manual-ssid')}
                            labelText={txt.ssidNetName}
                            placeholder={txt.ssidNetName}
                            onChange={e => this.#manualValChange('ssid', e.target.value)}
                        />
                        <Dropdown<Security>
                            id={$('manual-encryption-type')}
                            items={pb.wifiEncryptionTypeOptions}
                            selectedItem={manualEntryData.encryptionType || pb.EncryptionType.NONE}
                            label={txt.encryptionType}
                            titleText={txt.encryptionType}
                            onChange={this.#manualEncryptionSelect}
                            itemToString={this.#renderEncryptionType}
                            renderSelectedItem={this.#renderEncryptionType}
                        />
                        {!!manualEntryData.encryptionType &&
                        manualEntryData.encryptionType !== pb.EncryptionType.NONE ? (
                            <TextInput
                                id={$('password')}
                                labelText={txt.password}
                                placeholder={txt.password}
                                onChange={e => this.#manualValChange('password', e.target.value)}
                            />
                        ) : null}
                    </Form>
                </Fragment>
            );
        }

        // Enter password for selected network
        else if (selectedNetwork) {
            goBackBtn = { label: txt.back, onClick: this.#netDeselect };
            goNextBtn = {
                label: txt.confirm,
                onClick: () => {
                    const { ssid, encryptionType, password } = selectedNetwork;
                    this.#submit({
                        $typeName: 'braiins.bmc.web.SetWifiRequest',
                        ssid,
                        password,
                        encryptionType,
                    });
                },
                disabled: selectedNetwork.encryptionType !== pb.EncryptionType.NONE && !selectedNetwork.password,
            };

            content = (
                <Fragment>
                    <h1 className={css.title} children="Enter Wifi details manually" />

                    <Form className={css.manualForm}>
                        <Dropdown<pb.WifiNetwork>
                            id={$('selected-network-dropdown')}
                            className={css.netDropdownWrapper}
                            items={networks}
                            selectedItem={selectedNetwork}
                            label={txt.ssidNetName}
                            titleText={txt.ssidNetName}
                            onChange={x => this.#netSelect(x.selectedItem)}
                            itemToString={this.#netToString}
                            itemToElement={this.#netToElementMenu}
                            renderSelectedItem={this.#netToElementLabel}
                        />
                        {!!selectedNetwork.encryptionType &&
                        selectedNetwork.encryptionType !== pb.EncryptionType.NONE ? (
                            <TextInput
                                id={$('password')}
                                labelText={txt.password}
                                placeholder={txt.password}
                                value={selectedNetwork.password ?? ''}
                                onChange={e => this.#netSetPass(e.target.value)}
                            />
                        ) : null}
                    </Form>
                </Fragment>
            );
        }

        // Choose network from list
        else {
            goBackBtn = { label: txt.back, onClick: this.#goBack };
            goNextBtn = null;

            content = (
                <Fragment>
                    <h1 className={css.title} children="Select Wifi" />
                    <p className={css.note}>Select wifi you want to connect</p>

                    <div
                        className={css.netList}
                        children={
                            isLoading
                                ? this.#netloadingEntries
                                : networks.map((x, i) => (
                                      <WifiNetworkLine key={i} onClick={() => this.#netSelect(x)} net={x} />
                                  ))
                        }
                    />

                    <div className={css.links}>
                        <Link className={css.link} children="Other Networks" onClick={this.#manualEntryActivate} />
                        <Link className={css.link} children="Refresh" onClick={onReload} renderIcon={IconReload} />
                    </div>
                </Fragment>
            );
        }

        const footer: ReactElement[] = [];
        if (goBackBtn) {
            footer.push(
                <Button
                    key="a"
                    kind="secondary"
                    onClick={goBackBtn.onClick}
                    children={goBackBtn.label}
                    disabled={goBackBtn.disabled}
                />,
            );
        } else if (goNextBtn) footer.push(<span key="a" />);
        if (goNextBtn) {
            footer.push(
                <Button
                    key="b"
                    kind="primary"
                    onClick={goNextBtn.onClick}
                    children={goNextBtn.label}
                    disabled={goNextBtn.disabled}
                />,
            );
        } else if (goBackBtn) footer.push(<span key="b" />);

        return (
            <Layout header={<LogoHeader width="auto" height={18} style={{ width: 'auto' }} />} footer={footer}>
                <ProgressIndicator currentIndex={0} className={css.progress}>
                    <ProgressStep label="Wifi Settings" />
                    <ProgressStep label="Initial Setup" className={css.disabledTab} />
                </ProgressIndicator>
                {content}
            </Layout>
        );
    }
}

export function WifiConnect(props: WifiConnectProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
