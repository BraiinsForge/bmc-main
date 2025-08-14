import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import * as gen from '@/mocks';
import * as pb from '@/proto';

import { SectionSettings as Component, type SectionSettingsProps } from './SectionSettings';

function getArg<T>(name: string, value: T) {
    return {
        value,
        disabled: false,
        error: `${name} ${gen.lorem.generateWords(gen.number(3, 6))}`,
        onChange: action(`${name}.onChange`),
    };
}

export default {
    title: 'network/components/SectionSettings',
    component: Component,
    args: {
        status: [
            ['Uptime', '1h 13m 53s'],
            ['MAC Address', '00:C8:A2:8A:8E:6D'],
            ['RX', '162.92 KB (2151 Pkts.)'],
            ['TX', '3.13 MB (2605 Pkts.)'],
            ['IPv4', '10.34.2.2/24'],
        ],
        hostname: getArg('hostname', gen.hostname(1, 'local')),
        protocol: getArg('protocol', 'static'),
        staticAddress: getArg('staticConfig', '192.168.1.100'),
        staticNetmask: getArg('staticNetmask', '255.255.255.0'),
        staticGateway: getArg('staticGateway', '192.168.1.1'),
        staticDns: getArg('staticDnsServers', '8.8.8.8, 8.8.4.4'),

        // Wifi
        wifiActiveNetwork: {
            ...getArg('wifiActiveNetwork', {
                $typeName: 'braiins.bmc.web.WifiNetwork',
                signalStrength: pb.SignalStrength.MODERATE,
                connected: true,
                encryptionType: pb.EncryptionType.WPA2,
                ssid: 'MyWifi',
            }),
            async onConnectionRequest(...args: any[]) {
                action('wifiActiveNetwork.onConnectionRequest')(...args);
                return false;
            },
            onConnectionRequestCancel: action('wifiActiveNetwork.onConnectionRequestCancel'),
        },
        wifiAvailableNetworks: {
            isLoading: false,
            options: [
                {
                    $typeName: 'braiins.bmc.web.WifiNetwork',
                    signalStrength: pb.SignalStrength.STRONG,
                    encryptionType: pb.EncryptionType.WPA2,
                    ssid: 'Home_Network_5G',
                },
                {
                    $typeName: 'braiins.bmc.web.WifiNetwork',
                    signalStrength: pb.SignalStrength.STRONG,
                    encryptionType: pb.EncryptionType.WPA2,
                    ssid: 'IoT_Network',
                },
                {
                    $typeName: 'braiins.bmc.web.WifiNetwork',
                    signalStrength: pb.SignalStrength.MODERATE,
                    encryptionType: pb.EncryptionType.WPA2,
                    ssid: 'Neighbor_WiFi',
                },
                {
                    $typeName: 'braiins.bmc.web.WifiNetwork',
                    signalStrength: pb.SignalStrength.MODERATE,
                    encryptionType: pb.EncryptionType.WPA2_3,
                    ssid: 'Smart_Home_Net',
                },
                {
                    $typeName: 'braiins.bmc.web.WifiNetwork',
                    signalStrength: pb.SignalStrength.WEAK,
                    encryptionType: pb.EncryptionType.NONE,
                    ssid: 'Free_WiFi',
                },
            ],
            onRefresh: action('wifiAvailableNetworks.onRefresh'),
        },
        strings: { wifiConnect: 'Connect' },

        hasUnsavedChanges: true,
        onSave: action('onSave'),
        onReset: action('onReset'),
    } satisfies SectionSettingsProps,
} satisfies Meta<SectionSettingsProps>;

export function SectionGeneral(args: SectionSettingsProps) {
    return <Component {...args} />;
}
