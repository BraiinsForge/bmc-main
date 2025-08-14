import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import styled from '@emotion/styled';

import * as pb from '@/proto';
import { WifiNetworkLine as Component } from './WifiNetworkLine';

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
    title: 'components/WifiNetworkLine',
    component: Component,
} satisfies Meta;

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
                        <Component key={i} onClick={action('onClick')} net={x} variant="inline" />
                    ))}
                />
            </List>

            <List style={{ gap: 16 }}>
                <h5>WifiNetworkLine[variant=dropdown]</h5>
                <List
                    children={networks.map((x, i) => (
                        <Component key={i} onClick={action('onClick')} net={x} variant="dropdown" />
                    ))}
                />
            </List>

            <List style={{ gap: 16 }}>
                <h5>WifiNetworkLine.Skeleton</h5>
                <List>
                    <Component.Skeleton variant="inline" />
                    <Component.Skeleton variant="inline" />
                    <Component.Skeleton variant="inline" />
                    <Component.Skeleton variant="inline" />
                </List>
                <List>
                    <Component.Skeleton variant="dropdown" />
                    <Component.Skeleton variant="dropdown" />
                    <Component.Skeleton variant="dropdown" />
                    <Component.Skeleton variant="dropdown" />
                </List>
            </List>
        </List>
    );
}
WifiNetworkLine.storyName = 'WifiNetworkLine';
