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

import { useCallback, type HTMLAttributes, type KeyboardEvent } from 'react';
import { Key } from 'ts-key-enum';

// App
import * as pb from '@/proto';

// Components
import { WifiSignalStrength } from '@/components';
import {
    type CarbonIconType,
    Locked as IconLocked,
    Unlocked as IconUnlocked,
    Error as IconError,
} from '@carbon/react/icons';
import { SkeletonPlaceholder } from '@carbon/react';

// Styles
import cn from 'clsx';
import C from '@/styles/colors';
import css from './WifiNetworkLine.scss';

interface DataProps {
    net: pb.WifiNetwork;
    variant?: 'inline' | 'dropdown';
    onClick?(net: pb.WifiNetwork): void;
}
export interface WifiNetworkProps extends Omit<HTMLAttributes<HTMLDivElement>, keyof DataProps>, DataProps {}

export function WifiNetworkLine(props: WifiNetworkProps) {
    const { net, onClick, className, variant = 'inline', ...rest } = props;
    let SecurityIconComponent: CarbonIconType;
    let securityIconColor: string;

    // Unspecified or null
    if (!net.encryptionType) {
        SecurityIconComponent = IconError;
        securityIconColor = C.gray50;
    }

    // Insecure
    else if (net.encryptionType === pb.EncryptionType.NONE) {
        SecurityIconComponent = IconUnlocked;
        securityIconColor = C.alertRed;
    }

    // Fine, default case
    else {
        SecurityIconComponent = IconLocked;
        securityIconColor = 'currentColor';
    }

    const handleClick = useCallback(() => onClick?.(net), [onClick, net]);
    const handleKeyDown = useCallback(
        (e: KeyboardEvent): void => {
            if (e.key === ' ' || e.key === Key.Enter) handleClick();
        },
        [handleClick],
    );

    return (
        <div
            {...rest}
            role="listitem button"
            onClick={onClick ? handleClick : undefined}
            onKeyDown={onClick ? handleKeyDown : undefined}
            className={cn(css.root, css[`variant-${variant}`], onClick ? css.interactive : undefined, className)}
        >
            <div className={css.ssid} children={net.ssid} />
            <div className={css.icons}>
                <WifiSignalStrength size={16} state={net.signalStrength} />
                <SecurityIconComponent size={16} fill={securityIconColor} />
            </div>
        </div>
    );
}

WifiNetworkLine.Skeleton = (props: Pick<WifiNetworkProps, 'variant'>) => {
    const { variant = 'inline' } = props;

    return (
        <div className={cn(css.skeleton, css[`variant-${variant}`])}>
            <div className={css.root}>
                <SkeletonPlaceholder className={css.ssid} />
                <div className={css.icons}>
                    <WifiSignalStrength size={16} state={pb.SignalStrength.UNSPECIFIED} />
                </div>
            </div>
        </div>
    );
};
