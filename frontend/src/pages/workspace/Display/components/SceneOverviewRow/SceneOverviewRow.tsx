import type { DetailedHTMLProps, HTMLAttributes } from 'react';
import { useIntl } from 'react-intl';

// Lib
import { useID } from '@/lib/form';

// Components
import { Button } from '@/components';
import { Toggle, NumberInput } from '@carbon/react';
import { Draggable, TrashCan, Edit } from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './SceneOverviewRow.scss';

interface DataProps {
    id: number | string;

    enabled: boolean;
    onToggle(value: boolean): void;

    duration: string | number;
    onDurationChange(duration: string): void;

    onEdit(): void;
    onDelete(): void;

    preview: ReactNode;
    title: ReactNode;
    description: ReactNode;

    // DnD
    dndRootProps?: DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>;
    dndDragHandleProps?: DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>;
}
export interface SceneOverviewRowProps extends Omit<HTMLAttributes<HTMLDivElement>, keyof DataProps>, DataProps {}

export function SceneOverviewRow(props: SceneOverviewRowProps) {
    const {
        id,

        // State
        enabled,
        onToggle,

        duration,
        onDurationChange,

        onEdit,
        onDelete,

        preview,
        title,
        description,

        // DnD
        dndRootProps,
        dndDragHandleProps,

        // Pass-through
        className,
        ...rest
    } = props;
    const { formatMessage } = useIntl();
    const disabled: boolean = enabled === false;
    const $ = useID('scene', 'overview', 'row', id);

    return (
        <div {...rest} {...dndRootProps} className={cn(css.root, disabled && css.disabled, className)}>
            <div {...dndDragHandleProps} className={cn(css.dragHandle, dndDragHandleProps?.className)}>
                <Draggable />
            </div>

            <div className={css.toggle}>
                <Toggle
                    id={$('enabled')}
                    size="md"
                    labelA={formatMessage({ defaultMessage: 'Off' })}
                    labelB={formatMessage({ defaultMessage: 'On' })}
                    toggled={enabled}
                    onToggle={onToggle}
                />
            </div>

            <div className={css.preview} children={preview} />

            <div className={css.labels}>
                <div className={css.title} children={title} />
                <div className={css.details} children={description} />
            </div>

            <div className={css.duration}>
                <label htmlFor={$('duration')} children={formatMessage({ defaultMessage: 'Duration (s)' })} />
                <NumberInput
                    disabled={disabled}
                    id={$('duration')}
                    value={duration}
                    onChange={(_, s) => {
                        onDurationChange(String(s.value));
                    }}
                    step={1}
                />
            </div>

            <div className={css.actions}>
                <Button
                    size="sm"
                    kind="primary"
                    hasIconOnly
                    icon={Edit}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Edit' })}
                    onClick={onEdit}
                />
                <Button
                    size="sm"
                    kind="secondary"
                    hasIconOnly
                    icon={TrashCan}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Edit' })}
                    onClick={onDelete}
                />
            </div>
        </div>
    );
}
