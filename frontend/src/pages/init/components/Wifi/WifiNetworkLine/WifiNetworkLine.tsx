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
