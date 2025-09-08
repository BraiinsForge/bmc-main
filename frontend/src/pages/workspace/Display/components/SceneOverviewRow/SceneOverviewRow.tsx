import { useCallback } from 'react';
import type { DetailedHTMLProps, HTMLAttributes, SyntheticEvent } from 'react';
import { useIntl } from 'react-intl';

// Lib
import { useID } from '@/lib/form';
import { selfSelect } from '@/lib/react';

// Components
import { Button } from '@/components';
import { Toggle, NumberInput, Tag, type TagProps } from '@carbon/react';
import {
    Draggable as IconDraggable,
    TrashCan as IconDelete,
    Edit as IconEdit,
    Copy as IconClone,
} from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import css from './SceneOverviewRow.scss';

function applyDirectionToStringValue(value: string | number, direction: string): string {
    let res = Number.parseInt(value as string, 10);

    if (direction === 'up') res += 1;
    else if (direction === 'down') res -= 1;

    return String(res);
}

interface DataProps {
    id: string;

    enabled: boolean;
    onToggle(id: string, value: boolean): void;

    duration: Maybe<string | number>;
    durationDefault: string | number;
    onDurationChange(id: string, duration: string): void;

    onEdit(id: string): void;
    onClone(id: string): void;
    onDelete(id: string): void;

    preview: ReactNode;
    title: ReactNode;
    tag?: null | {
        text: NonNullable<ReactNode>;
        type: TagProps<'div'>['type'];
        className?: string;
        style?: CSSProperties;
    };
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
        durationDefault,
        onDurationChange,

        onEdit,
        onClone,
        onDelete,

        preview,
        title,
        tag,
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
        (event: SyntheticEvent, info: { direction: string }) => {
            const target = event.target;

            // The value from CDS is not used because if the input is empty and has
            // a minimum value, the minimum value is given instead of empty string…
            //
            // This fucks us here, because we need real value without processing.
            // This is also because the given `direction` string is basically useless
            // as it's `down` when the input is just cleared.
            //
            // The fucky input selection is here because the change handler
            // can be called from following events / elements:
            //  - MouseEvent<HTMLButtonElement>
            //  - FocusEvent<HTMLInputElement>
            //  - KeyboardEvent<HTMLInputElement>
            const input: Maybe<HTMLInputElement> = (target as HTMLElement)
                .closest('.cds--form-item')
                ?.querySelector('input[type="number"]');

            // The value is then obtained directly from the input element.
            const valueRaw: string = input?.value ?? '';
            const value: string =
                target instanceof HTMLButtonElement || 'keyCode' in event
                    ? applyDirectionToStringValue(valueRaw, info.direction)
                    : valueRaw;

            // NOOP: Some events are fired even though no change is possible
            // eg.: example: `value === min` and minus button is pressed
            if (value === String(duration)) return;

            // Removing the value (number => '')
            if (!!duration && value === '') onDurationChange(id, '');
            //
            // Empty value (default used) and something got triggered
            // => use defaultValue and apply the reported operation to it
            else if (!duration) onDurationChange(id, applyDirectionToStringValue(durationDefault, info.direction));
            //
            // default case, just pass the value upward
            else onDurationChange(id, value);

            if (input) input.value = '';
        },
        [id, onDurationChange, duration, durationDefault],
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
                <div className={css.title}>
                    <span children={title} />
                    {tag ? (
                        <Tag
                            type={tag.type}
                            children={tag.text}
                            style={tag.style}
                            className={cn(css.tag, tag.className)}
                            size="sm"
                        />
                    ) : null}
                </div>
                <div className={css.details} children={description} />
            </div>

            <div className={css.duration}>
                <label htmlFor={$('duration')} children={formatMessage({ defaultMessage: 'Duration (s)' })} />
                <NumberInput
                    disabled={disabled}
                    id={$('duration')}
                    min={1}
                    step={1}
                    allowEmpty
                    placeholder={String(durationDefault)}
                    value={duration ?? ''}
                    onChange={handleDurationChange}
                    onFocus={selfSelect}
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
                    title={formatMessage({ defaultMessage: 'Delete' })}
                    onClick={handleDelete}
                />
            </div>
        </div>
    );
}

export interface SceneOverviewRowSkeletonProps extends HTMLAttributes<HTMLDivElement> {
    rowCount?: number;
}
export function SceneOverviewRowSkeleton(props: SceneOverviewRowSkeletonProps) {
    const { rowCount, className, ...rest } = props;

    if (rowCount != null && rowCount > 1) {
        const opacityBase = 0.6;
        const opacityStepSize = opacityBase / rowCount;
        const items: ReactNode[] = Array.from({ length: rowCount }, (_, i) => (
            <SceneOverviewRowSkeleton key={i} style={{ opacity: opacityBase - opacityStepSize * i }} />
        ));

        return <div {...rest} children={items} className={cn(css.skeletonsGroup, className)} />;
    }

    return (
        <div {...rest} role="listitem" aria-hidden="true" className={cn(css.skeleton, className)}>
            <div className={css.a} />
            <div className={css.b1} />
            <div className={css.b2} />
            <div className={css.c} />
            <div className={css.d} />
        </div>
    );
}
