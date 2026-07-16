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

import { Fragment, useCallback, useState, useRef } from 'react';
import { useIntl } from 'react-intl';
import { mergeRefs } from '@/lib/react';

// App
import * as pb from '@/proto';
import * as fn from '../../fn';

// DnD
import {
    DndContext,
    DragOverlay,
    PointerSensor,
    pointerWithin,
    useSensor,
    useSensors,
    type DataRef,
    type DragEndEvent,
    type DropAnimation,
    useDroppable,
    useDraggable,
} from '@dnd-kit/core';

const dragOverlayDropAnimation: DropAnimation = {
    duration: 150,
    easing: 'cubic-bezier(0.18, 0.67, 0.6, 1.22)',
};

// Components
import {
    Draggable as IconWidgetMove,
    SettingsAdjust as IconWidgetAdjust,
    SubtractAlt as IconWidgetDelete,
    AddAlt as IconWidgetAdd,
} from '@carbon/react/icons';
import { Tooltip } from '@/components';

// Styles
import cn from 'clsx';
import css from './CombinedSceneView.scss';

interface DraggableData {
    getStartRect(): null | DOMRect;
}
type DroppableData = fn.Located;

export interface CombinedSceneViewProps {
    widgets: pb.Widget[];
    manifests: pb.ManifestLookup;

    onWidgetMove(source: pb.Widget, target: fn.Located): void;
    onWidgetAdd(position: pb.WidgetPosition): void;
    onWidgetEdit(id: string): void;
    onWidgetRemove(id: string): void;

    style?: CSSProperties;
    className?: string;
}
interface ViewProps extends CombinedSceneViewProps {
    validDropSlots: fn.ValidDropSlots;
}

/**
 * Just the visual view with DnD shit delegated
 * to the wrapper component to somewhat simplify
 * the code of this.
 * */
function View(props: ViewProps) {
    const {
        widgets,
        manifests,
        validDropSlots,

        // Handlers
        onWidgetAdd,
        onWidgetEdit,
        onWidgetRemove,

        // Pass-through
        className,
        style,
    } = props;

    const data = fn.injectPlaceholdersToUnoccupiedSlots(widgets);

    return (
        <div className={cn(css.root, className)} style={style}>
            <div
                className={css.grid}
                children={data.map((w, i) => {
                    const id = w.id;
                    const pos = w.position as pb.WidgetPosition;
                    const size = w.size as pb.WidgetSize;

                    if ('isPlaceholder' in w && w.isPlaceholder) {
                        return (
                            <Widget
                                id={id}
                                key={i}
                                placeholder
                                position={pos}
                                size={size}
                                onAdd={() => onWidgetAdd(pos)}
                                validDropSlots={validDropSlots}
                            />
                        );
                    }

                    const widget = w as pb.Widget;
                    const title = pb.widgetTitle(widget, manifests) || 'N/A';

                    return (
                        <Widget
                            id={id}
                            key={i}
                            // Layout
                            size={size}
                            position={pos}
                            // Content
                            title={title}
                            subtitle={pb.widgetDescription(widget, manifests)}
                            // Modification
                            validDropSlots={validDropSlots}
                            onEdit={() => onWidgetEdit(id)}
                            onDelete={() => onWidgetRemove(id)}
                        />
                    );
                })}
            />
        </div>
    );
}

interface WidgetProps {
    isPreview?: boolean;
    previewStyle?: CSSProperties;
    validDropSlots: fn.ValidDropSlots;

    id: string;
    position: pb.WidgetPosition;
    size: pb.WidgetSize;
    placeholder?: boolean;

    // Existing widget attributes
    onEdit?(): void;
    onDelete?(): void;

    // Placeholder cell attributes
    onAdd?(position: pb.WidgetPosition): void;
    title?: null | string;
    subtitle?: null | string;
}
function Widget(props: WidgetProps) {
    const {
        isPreview,
        previewStyle,
        validDropSlots,

        id,
        position,
        size,
        placeholder,
        onEdit,
        onDelete,

        // Content
        onAdd,
        title,
        subtitle,
    } = props;
    const { row, col } = position;

    const intl = useIntl();
    const ref = useRef<null | HTMLDivElement>(null);
    const handleAdd = useCallback(() => {
        if (onAdd) onAdd(fn.pos(row, col));
    }, [row, col, onAdd]);

    const dropZoneKey = fn.dropSlotKey(row, col);
    const canBeDropped: boolean = validDropSlots.has(dropZoneKey);

    //
    // D'n'D hooks
    //

    const $drop = useDroppable({
        id,
        // Currently we don't do swaps on backend
        disabled: !placeholder || !canBeDropped,
        data: { id, position, size } satisfies DroppableData,
    });
    const $drag = useDraggable({
        id,
        // Placeholders are not draggable
        disabled: !!placeholder,
        data: {
            getStartRect: () => ref?.current?.getBoundingClientRect() ?? null,
        } satisfies DraggableData,
    });
    const disableTooltips = isPreview || $drag.active;

    // When we start moving a non-small widget, we want to let all
    // of it's underlying slots to become available as drop-targets.
    //
    // This amounts to exploding the widget into individual slots
    // and creating a new placeholder widget for each.
    if (!placeholder && size !== pb.WidgetSize.SMALL && $drag.isDragging) {
        return fn.explodeWidgetIntoAtoms({ id, position, size }).map((x, i) => {
            return (
                <Widget
                    key={i}
                    placeholder
                    id={x.id}
                    position={x.position}
                    size={x.size}
                    onAdd={onAdd}
                    validDropSlots={validDropSlots}
                />
            );
        });
    }

    // Grid area style construction
    const gridAreaName: string = `r${row + 1}c${col + 1}`;
    const gridArea = `${gridAreaName} / ${gridAreaName} / span ${fn.WIDGET_SIZE_TO_SPAN[size || pb.WidgetSize.SMALL].rows} / span ${fn.WIDGET_SIZE_TO_SPAN[size || pb.WidgetSize.SMALL].cols}`;

    return (
        <div
            ref={mergeRefs($drop.setNodeRef, $drag.setNodeRef, ref)}
            className={cn(
                css.widget,
                isPreview && css.isPreview,
                !placeholder && css.taken,
                $drag.isDragging && css.isDragging,
                !$drag.isDragging && $drop.isOver && css.isDropHovered,
                !!$drag.active && canBeDropped === false && css.isInvalidTarget,
            )}
            style={{ gridArea, ...(isPreview ? previewStyle : {}) }}
        >
            {!placeholder && !$drag.active ? (
                <Tooltip
                    // If we don't trash the tooltips when DnD state changes,
                    // they are shown / keep showing in a buggy manner
                    key={`move-${$drag.isDragging}`}
                    placement="bottom"
                    delayShow={800}
                    content={intl.formatMessage({ defaultMessage: 'Move Widget' })}
                    render={r => (
                        <div
                            {...$drag.listeners}
                            {...$drag.attributes}
                            // We have to "disable" the tooltips when DnD is active to prevent visual glitching
                            ref={disableTooltips ? null : r}
                            className={css.widgetDragHandle}
                        >
                            <IconWidgetMove size={16} />
                        </div>
                    )}
                />
            ) : null}

            {onEdit && !$drag.active ? (
                <Tooltip
                    // If we don't trash the tooltips when DnD state changes,
                    // they are shown / keep showing in a buggy manner
                    key={`edit-${$drag.isDragging}`}
                    placement="bottom"
                    delayShow={800}
                    content={intl.formatMessage({ defaultMessage: 'Edit Widget' })}
                    render={r => (
                        <button
                            // We have to "disable" the tooltips when DnD is active to prevent visual glitching
                            ref={disableTooltips ? null : r}
                            type="button"
                            className={css.widgetEditButton}
                            onClick={onEdit}
                        >
                            <IconWidgetAdjust size={16} />
                        </button>
                    )}
                />
            ) : null}

            {onDelete && !$drag.active ? (
                <Tooltip
                    // If we don't trash the tooltips when DnD state changes,
                    // they are shown / keep showing in a buggy manner
                    key={`remove-${$drag.isDragging}`}
                    placement="bottom"
                    delayShow={800}
                    content={intl.formatMessage({ defaultMessage: 'Remove Widget' })}
                    render={r => (
                        <button
                            // We have to "disable" the tooltips when DnD is active to prevent visual glitching
                            ref={disableTooltips ? null : r}
                            type="button"
                            className={css.widgetDeleteButton}
                            onClick={onDelete}
                        >
                            <IconWidgetDelete size={16} />
                        </button>
                    )}
                />
            ) : null}

            <div className={css.widgetContent}>
                {onAdd && !$drag.active ? (
                    <button
                        type="button"
                        className={css.widgetAddButtom}
                        onClick={handleAdd}
                        children={<IconWidgetAdd size={16} />}
                    />
                ) : (
                    <Fragment>
                        {/*
                        The content is duplicated into title because the node width
                        can be very limited and it's "text-overflow: ellipsis".

                        This means that it will truncate and we gotta show
                        the full content somehow.
                        */}
                        {title ? <div className={css.widgetTitle} children={title} title={title} /> : null}
                        {subtitle ? <div className={css.widgetSubtitle} children={subtitle} title={subtitle} /> : null}
                    </Fragment>
                )}
            </div>
        </div>
    );
}

export function CombinedSceneView(props: CombinedSceneViewProps) {
    const { widgets, manifests, onWidgetMove } = props;

    const sensors = useSensors(useSensor(PointerSensor));
    const [activeId, setActiveId] = useState<null | string>(null);
    const [activeRect, setActiveRect] = useState<null | DOMRect>(null);
    const activeWidget = (activeId ? widgets.find(w => w.id === activeId) : null) as Maybe<Required<pb.Widget>>;
    const [validDropSlots, setValidDropSlots] = useState<fn.ValidDropSlots>(new Set());

    const handleDragStart = useCallback(
        (e: DragEndEvent) => {
            const active = e.active;
            const id = String(active.id);
            const data = (active.data as DataRef<DraggableData>).current;
            const widget = widgets.find(w => w.id === id);

            setActiveId(id);

            // Pre-calculate the valid drop slots for the active widget
            if (widget) setValidDropSlots(fn.getValidDropSlots(widgets, widget));

            // Capture the dimensions of the dragged element
            const rect = data?.getStartRect();
            if (rect) setActiveRect(rect);
        },
        [widgets],
    );
    const handleDragEnd = useCallback(
        (e: DragEndEvent) => {
            // Reset the info about active widget and related data
            setActiveId(null);
            setActiveRect(null);
            setValidDropSlots(new Set());

            const idActive: string = String(e.active.id);
            const idOver: string = String(e.over?.id);
            if (idActive === idOver) return;

            const source: Maybe<pb.Widget> = widgets.find(x => x.id === idActive);
            const target: Maybe<fn.Located> = (e.over?.data as DataRef<DroppableData>).current;

            if (!source || !target) {
                console.warn('Invalid drag-end state, source or target not found', { source, target });
                return;
            }
            onWidgetMove(source, target);
        },
        [widgets, onWidgetMove],
    );

    return (
        <DndContext
            sensors={sensors}
            collisionDetection={pointerWithin}
            onDragEnd={handleDragEnd}
            onDragStart={handleDragStart}
        >
            <View {...props} validDropSlots={validDropSlots} />

            <DragOverlay dropAnimation={dragOverlayDropAnimation}>
                {activeWidget ? (
                    <Widget
                        isPreview
                        validDropSlots={validDropSlots}
                        previewStyle={activeRect ? { width: activeRect.width, height: activeRect.height } : undefined}
                        id={activeWidget.id}
                        position={activeWidget.position}
                        size={activeWidget.size}
                        title={pb.widgetTitle(activeWidget, manifests) ?? 'N/A'}
                        subtitle={pb.widgetDescription(activeWidget, manifests)}
                    />
                ) : null}
            </DragOverlay>
        </DndContext>
    );
}
