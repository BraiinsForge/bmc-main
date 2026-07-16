// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

import type { SVGProps } from 'react';

import * as pb from '@/proto';
import { WifiOff } from '@carbon/react/icons';
import { Tick } from '@/components/Tick';

import cn from 'clsx';
import css from './WifiSignalStrength.scss';

type WifiState = 'offline' | 'low' | 'fair' | 'full' | 'scanning';
export interface WifiSignalStrengthProps extends SVGProps<SVGSVGElement> {
    size?: number;
    state: WifiState | pb.SignalStrength;
}

const signalStrengthToWifiIconState: Record<pb.SignalStrength, WifiState> = {
    [pb.SignalStrength.UNSPECIFIED]: 'scanning',
    [pb.SignalStrength.WEAK]: 'low',
    [pb.SignalStrength.MODERATE]: 'fair',
    [pb.SignalStrength.STRONG]: 'full',
} as const;

const scanFrames = ['low', 'fair', 'full', 'fair'] as WifiState[];
function getScanFrame(i: number): WifiState {
    return scanFrames[(i + 1) % scanFrames.length];
}

export function WifiSignalStrength(props: WifiSignalStrengthProps) {
    const { state, size, ...rest } = props;
    const $ =
        state in signalStrengthToWifiIconState
            ? (signalStrengthToWifiIconState[state as pb.SignalStrength] as WifiState)
            : (state as WifiState);

    switch ($) {
        case 'scanning':
            return (
                <Tick
                    intervalMs={300}
                    render={n => <WifiSignalStrength {...rest} size={size} state={getScanFrame(n)} />}
                />
            );

        case 'offline':
            return <WifiOff className={cn(css.svg, css.offline)} size={size} {...rest} />;

        case 'low':
        case 'fair':
        case 'full':
            // The SVG below inlines Carbon Design System's Wifi icon to keep its paths stable.
            // Copyright IBM Corp., Carbon Design System, licensed under Apache-2.0.
            return (
                <svg
                    {...rest}
                    className={cn(css.svg, css[state as 'low' | 'fair' | 'full'], props.className)}
                    focusable="false"
                    preserveAspectRatio="xMidYMid meet"
                    fill="currentColor"
                    width={size}
                    height={size}
                    viewBox="0 0 32 32"
                    aria-hidden="true"
                    xmlns="http://www.w3.org/2000/svg"
                >
                    <path d="M30,10.7412a19.94,19.94,0,0,0-28,0v.0225L3.4043,12.168a17.9336,17.9336,0,0,1,25.1811-.01L30,10.7432Z" />
                    <path d="M6.229,14.9927l1.4136,1.4135a11.955,11.955,0,0,1,16.7041-.01L25.76,14.9829a13.9514,13.9514,0,0,0-19.5313.01Z" />
                    <path d="M10.47,19.2334l1.4136,1.4131a5.9688,5.9688,0,0,1,8.2229-.0093L21.52,19.2236a7.9629,7.9629,0,0,0-11.05.01Z" />
                    <circle cx="16" cy="25" r="2" />
                </svg>
            );
    }
}
