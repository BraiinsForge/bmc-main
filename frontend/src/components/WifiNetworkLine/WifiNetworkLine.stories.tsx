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
