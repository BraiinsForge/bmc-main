import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import styled from '@emotion/styled';

import * as pb from '@/proto';
import { WifiConnect as Component, type WifiConnectProps } from './WifiConnect';
import { WifiNetworkLine as WifiComponent } from './WifiNetworkLine';
import { DoneScene as Done } from './DoneScene';

const networks: Array<pb.WifiNetwork> = [
    {
        $typeName: 'braiins.bmc.web.WifiNetwork',
        ssid: 'Home Network',
        encryptionType: pb.EncryptionType.NONE,
        signalStrength: pb.SignalStrength.MODERATE,
    },
    {
        $typeName: 'braiins.bmc.web.WifiNetwork',
        ssid: 'UPC_16a4546',
        encryptionType: pb.EncryptionType.WPA2,
        signalStrength: pb.SignalStrength.STRONG,
    },
    {
        $typeName: 'braiins.bmc.web.WifiNetwork',
        ssid: 'O2_N4s5',
        encryptionType: pb.EncryptionType.WEP_SHARED,
        signalStrength: pb.SignalStrength.WEAK,
    },
    {
        $typeName: 'braiins.bmc.web.WifiNetwork',
        ssid: 'UPC_5s849',
        encryptionType: pb.EncryptionType.UNSPECIFIED,
        signalStrength: pb.SignalStrength.UNSPECIFIED,
    },
];

export default {
    title: 'init/WifiConnect',
    component: Component,
} satisfies Meta;

export function WifiConnect(args: WifiConnectProps) {
    return <Component {...args} />;
}
WifiConnect.args = {
    onBack: action('onBack'),
    async onSubmit(...args) {
        action('onSubmit')(...args);
        return true;
    },
    networks: networks,
    onSelect: action('onSelect'),
    onReload: action('onReload'),
    isLoading: false,
} satisfies WifiConnectProps;

export function DoneScene() {
    return <Done />;
}

export function WifiNetworkLine() {
    const List = styled.div`
        width: 500px;
        display: inline-flex;
        flex-direction: column;
    `;

    return (
        <List style={{ gap: 16, padding: 16 }}>
            <List style={{ gap: 16 }}>
                <h5>WifiNetworkLine[variant=inline]</h5>
                <List
                    children={networks.map((x, i) => (
                        <WifiComponent key={i} onClick={action('onClick')} net={x} variant="inline" />
                    ))}
                />
            </List>

            <List style={{ gap: 16 }}>
                <h5>WifiNetworkLine[variant=dropdown]</h5>
                <List
                    children={networks.map((x, i) => (
                        <WifiComponent key={i} onClick={action('onClick')} net={x} variant="dropdown" />
                    ))}
                />
            </List>

            <List style={{ gap: 16 }}>
                <h5>WifiNetworkLine.Skeleton</h5>
                <List>
                    <WifiComponent.Skeleton variant="inline" />
                    <WifiComponent.Skeleton variant="inline" />
                    <WifiComponent.Skeleton variant="inline" />
                    <WifiComponent.Skeleton variant="inline" />
                </List>
                <List>
                    <WifiComponent.Skeleton variant="dropdown" />
                    <WifiComponent.Skeleton variant="dropdown" />
                    <WifiComponent.Skeleton variant="dropdown" />
                    <WifiComponent.Skeleton variant="dropdown" />
                </List>
            </List>
        </List>
    );
}
WifiNetworkLine.storyName = '- WifiNetworkLine';
