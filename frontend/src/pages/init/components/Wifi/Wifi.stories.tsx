import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { WifiConnect as Component, type WifiConnectProps } from './WifiConnect';
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
