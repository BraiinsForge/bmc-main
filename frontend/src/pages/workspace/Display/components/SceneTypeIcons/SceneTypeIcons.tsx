import type { HTMLAttributes } from 'react';
import { useIntl } from 'react-intl';

// Components
import { Deck as IconDeck, Tooltip } from '@/components';
import { type CarbonIconType, Asleep as IconNight, Cloud as IconCloud } from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './SceneTypeIcon.scss';

export interface SceneTypeIconsProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
    night?: boolean;
    local?: boolean;
    cloud?: boolean;
}

interface PillProps {
    icon: CarbonIconType;
    text: string;
    className: string;
}
function Pill(props: PillProps) {
    const { icon: Icon, text, className } = props;
    return (
        <Tooltip
            placement="bottom"
            content={text}
            render={r => (
                <div className={cn(css.pill, className)} ref={r}>
                    <Icon size={16} />
                </div>
            )}
        />
    );
}

export function SceneTypeIcons(props: SceneTypeIconsProps) {
    const { night, local, cloud, className, ...rest } = props;
    const { formatMessage } = useIntl();

    const content: ReactNode[] = [];
    if (night) {
        content.push(
            <Pill
                key="night"
                icon={IconNight}
                className={css.night}
                text={formatMessage({
                    defaultMessage:
                        'Night Mode - First widget stays displayed during the night mode, when the rotation is disabled.',
                })}
            />,
        );
    }
    if (cloud) {
        content.push(
            <Pill
                key="cloud"
                icon={IconCloud}
                className={css.cloud}
                text={formatMessage({ defaultMessage: 'Remote widget - displays live data from external source' })}
            />,
        );
    }
    if (local) {
        content.push(
            <Pill
                key="local"
                icon={IconDeck}
                className={css.local}
                text={formatMessage({ defaultMessage: 'Local widget - displays data stored or generated locally' })}
            />,
        );
    }

    return <div {...rest} className={cn(className, css.root)} children={content} />;
}
