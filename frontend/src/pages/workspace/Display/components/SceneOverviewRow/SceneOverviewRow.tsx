import { type DetailedHTMLProps, type HTMLAttributes, useCallback } from 'react';
import { useIntl } from 'react-intl';

// Lib
import { useID } from '@/lib/form';

// Components
import { Button } from '@/components';
import { Toggle, NumberInput } from '@carbon/react';
import {
    Draggable as IconDraggable,
    TrashCan as IconDelete,
    Edit as IconEdit,
    Copy as IconClone,
} from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './SceneOverviewRow.scss';

interface DataProps {
    id: string;

    enabled: boolean;
    onToggle(id: string, value: boolean): void;

    duration: string | number;
    onDurationChange(id: string, duration: string): void;

    onEdit(id: string): void;
    onClone(id: string): void;
    onDelete(id: string): void;

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
        onClone,
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

    const handleToggle = useCallback((value: boolean) => onToggle(id, value), [id, onToggle]);
    const handleDurationChange = useCallback(
        (_: any, s: { value: number | string }) => {
            onDurationChange(id, String(s.value));
        },
        [id, onDurationChange],
    );
    const handleEdit = useCallback(() => onEdit(id), [id, onEdit]);
    const handleClone = useCallback(() => onClone(id), [id, onClone]);
    const handleDelete = useCallback(() => onDelete(id), [id, onDelete]);

    return (
        <div {...rest} {...dndRootProps} className={cn(css.root, disabled && css.disabled, className)}>
            <div
                {...dndDragHandleProps}
                className={cn(css.dragHandle, dndDragHandleProps?.className)}
                children={<IconDraggable />}
            />

            <div className={css.toggle}>
                <Toggle
                    id={$('enabled')}
                    size="md"
                    labelA={formatMessage({ defaultMessage: 'Off' })}
                    labelB={formatMessage({ defaultMessage: 'On' })}
                    toggled={enabled}
                    onToggle={handleToggle}
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
                    onChange={handleDurationChange}
                    step={1}
                />
            </div>

            <div className={css.actions}>
                <Button
                    id={$('edit')}
                    size="sm"
                    kind="primary"
                    hasIconOnly
                    icon={IconEdit}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Edit' })}
                    onClick={handleEdit}
                />
                <Button
                    id={$('clone')}
                    size="sm"
                    kind="secondary"
                    hasIconOnly
                    icon={IconClone}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Clone' })}
                    onClick={handleClone}
                />
                <Button
                    id={$('delete')}
                    size="sm"
                    kind="secondary"
                    hasIconOnly
                    icon={IconDelete}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Edit' })}
                    onClick={handleDelete}
                />
            </div>
        </div>
    );
}
