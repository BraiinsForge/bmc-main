import { useRef } from 'react';
import { useIntl } from 'react-intl';
import { type CarbonIconType, useSizeSelector } from '@/lib/react';

// Components
import * as Icons from '../icons';
import { Button } from '@/components';
import { Add as IconAdd } from '@carbon/react/icons';

// styles
import css from './FormSceneSelect.scss';

export enum SceneKind {
    Combined = 'combined',
    Clocks = 'clocks',
    Ticker = 'ticker',
    Pool = 'pool',
    Manager = 'manager',
}

export interface FormSceneSelectProps {
    onClick(kind: SceneKind): void;
}

export function FormSceneSelect(props: FormSceneSelectProps) {
    const { formatMessage } = useIntl();
    const { onClick } = props;

    return (
        <section role="list" className={css.root}>
            <Row
                icon={Icons.WidgetCombined}
                title={formatMessage({ defaultMessage: 'Combined Scene' })}
                description={formatMessage({
                    defaultMessage: 'Combined scene displaying multiple configurable modules that can be adjusted.',
                })}
                onClick={() => onClick(SceneKind.Combined)}
            />

            <Row
                icon={Icons.WidgetClocks}
                title={formatMessage({ defaultMessage: 'Clocks' })}
                description={formatMessage({
                    defaultMessage: 'You can choose between types of clocks - Flip, Digital, Analog',
                })}
                onClick={() => onClick(SceneKind.Clocks)}
            />

            <Row
                icon={Icons.WidgetTicker}
                title={formatMessage({ defaultMessage: 'Ticker' })}
                description={formatMessage({
                    defaultMessage: 'BTC or Stock price adjusted in 5min intervals.  Few types - List, Big Price',
                })}
                onClick={() => onClick(SceneKind.Ticker)}
            />

            <Row
                icon={Icons.WidgetPool}
                title={formatMessage({ defaultMessage: 'Braiins Pool' })}
                description={formatMessage({
                    defaultMessage: 'Combined scene displaying multiple configurable modules that can be adjusted.',
                })}
                onClick={() => onClick(SceneKind.Pool)}
            />

            <Row
                icon={Icons.WidgetManager}
                title={formatMessage({ defaultMessage: 'Braiins Manager' })}
                description={formatMessage({
                    defaultMessage:
                        'Connect your Braiins Manager account and get real-time stats for your mining operation.',
                })}
                onClick={() => onClick(SceneKind.Manager)}
            />
        </section>
    );
}

interface RowProps {
    icon: CarbonIconType;
    title: string;
    description: string;
    onClick(): void;
}
function Row(props: RowProps) {
    const { formatMessage } = useIntl();

    const ref = useRef<HTMLDivElement>(null);
    const isMobileLayout: boolean = useSizeSelector(ref, s => !!s && s.width <= 600);

    const { icon: Icon, title, description, onClick } = props;

    return (
        <div ref={ref} className={css.row} role="listitem">
            <div className={css.rowIcon} children={<Icon size={56} />} />
            <div className={css.rowTitle} children={title} />
            <div className={css.rowDescription} children={description} />
            <div className={css.rowAction}>
                <Button
                    kind="primary"
                    icon={IconAdd}
                    size={isMobileLayout ? 'md' : 'lg'}
                    children={isMobileLayout ? null : formatMessage({ defaultMessage: 'Add Scene' })}
                    onClick={onClick}
                />
            </div>
        </div>
    );
}
