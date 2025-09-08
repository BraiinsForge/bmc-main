import { useRef } from 'react';
import { useIntl } from 'react-intl';

import { type CarbonIconType, useSizeSelector } from '@/lib/react';

// App
import { getID } from '../const';
import type * as pb from '@/proto';

// Components
import * as Icons from '@/components/images/icons';
import { Button, type ButtonProps, ModalCustom } from '@/components';
import { Add as IconAdd } from '@carbon/react/icons';

// styles
import css from './FormSceneSelect.scss';

export type SceneWidgetKind = ProtoOneofCase<pb.WidgetKind['value']>;
export type SceneKind = 'combined' | SceneWidgetKind;

export interface FormSceneSelectProps {
    variant: 'widget' | 'scene';

    isOpen: boolean;
    onClose(): void;
    onSelection(kind: SceneKind): void;
}

const $ = getID('scene-select-kind').get;
export function FormSceneSelect(props: FormSceneSelectProps) {
    const { formatMessage } = useIntl();
    const { onSelection, isOpen, onClose, variant } = props;

    return (
        <ModalCustom
            id={$('modal')}
            open={isOpen}
            size="sm"
            title={
                variant === 'scene'
                    ? formatMessage({ defaultMessage: 'Add New Display Scene' })
                    : formatMessage({ defaultMessage: 'Add New Widget' })
            }
            selectorPrimaryFocus="[role=list] [role=button]"
            onClose={onClose}
            cancelBodyOverflowShadow
        >
            <section role="list" className={css.root}>
                {variant === 'scene' ? (
                    <Row
                        variant={variant}
                        icon={Icons.WidgetCombined}
                        title={formatMessage({ defaultMessage: 'Combined Scene' })}
                        description={formatMessage({
                            defaultMessage:
                                'Combined scene displaying multiple configurable modules that can be adjusted.',
                        })}
                        onClick={() => onSelection('combined')}
                    />
                ) : null}

                <Row
                    variant={variant}
                    icon={Icons.WidgetClocks}
                    title={formatMessage({ defaultMessage: 'Clocks' })}
                    description={formatMessage({
                        defaultMessage: 'You can choose between types of clocks - Flip, Digital, Analog',
                    })}
                    onClick={() => onSelection('clock')}
                />

                <Row
                    variant={variant}
                    icon={Icons.WidgetTicker}
                    title={formatMessage({ defaultMessage: 'Ticker' })}
                    description={formatMessage({ defaultMessage: 'BTC price adjusted in 5min intervals.' })}
                    onClick={() => onSelection('tickerBtc')}
                />

                <Row
                    variant={variant}
                    icon={Icons.WidgetBlockHeight}
                    title={formatMessage({ defaultMessage: 'Block Height' })}
                    description={formatMessage({
                        defaultMessage: 'Combined scene displaying multiple configurable modules that can be adjusted.',
                    })}
                    onClick={() => onSelection('blockHeight')}
                />

                {/*
                <Row
                    variant={variant}
                    icon={Icons.WidgetPool}
                    title={formatMessage({ defaultMessage: 'Braiins Pool' })}
                    description={formatMessage({ defaultMessage: 'Combined scene displaying multiple configurable modules that can be adjusted.' })}
                    onClick={() => onSelection('pool')}
                />

                <Row
                    variant={variant}
                    icon={Icons.WidgetManager}
                    title={formatMessage({ defaultMessage: 'Braiins Manager' })}
                    description={formatMessage({ defaultMessage: 'Connect your Braiins Manager account and get real-time stats for your mining operation.' })}
                    onClick={() => onSelection('manager')}
                />
                */}
            </section>
        </ModalCustom>
    );
}

interface RowProps {
    variant: FormSceneSelectProps['variant'];

    icon: CarbonIconType;
    title: string;
    description: null | string;
    onClick(): void;
}
function Row(props: RowProps) {
    const { icon: Icon, title, description, onClick, variant } = props;
    const { formatMessage } = useIntl();

    const ref = useRef<HTMLDivElement>(null);
    const isMobileLayout: boolean = useSizeSelector(ref, s => !!s && s.width <= 600);

    const actionLabel: string =
        variant === 'scene'
            ? formatMessage({ defaultMessage: 'Add Scene' })
            : formatMessage({ defaultMessage: 'Add Widget' });
    const buttonProps: Partial<ButtonProps> = {};
    if (isMobileLayout) {
        buttonProps.title = actionLabel;
        buttonProps.hasIconOnly = true;
    } else {
        buttonProps.children = actionLabel;
    }

    return (
        <div ref={ref} className={css.row} role="listitem">
            <div className={css.rowIcon} children={<Icon size={56} />} />
            <div className={css.rowTitle} children={title} />
            <div className={css.rowDescription} children={description} />
            <div className={css.rowAction}>
                <Button
                    id={$('add-scene')}
                    kind="primary"
                    icon={IconAdd}
                    size={isMobileLayout ? 'md' : 'lg'}
                    tooltipPosition="bottom"
                    tooltipAlignment="end"
                    onClick={onClick}
                    {...buttonProps}
                />
            </div>
        </div>
    );
}
