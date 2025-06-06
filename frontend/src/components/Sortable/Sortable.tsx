import type { RefCallback } from 'react';

// Drag and drop
import { CSS } from '@dnd-kit/utilities';
import {
    DndContext,
    closestCenter,
    KeyboardSensor,
    PointerSensor,
    useSensor,
    useSensors,
    type DragEndEvent,
    type DraggableAttributes,
} from '@dnd-kit/core';
import {
    useSortable,
    arrayMove,
    SortableContext,
    sortableKeyboardCoordinates,
    verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import type { SyntheticListenerMap } from '@dnd-kit/core/dist/hooks/utilities';

// Styles
import css from './Sortable.scss';
import cn from 'clsx';

//
// Item
//

interface Datum {
    id: string | number;
}
export interface RenderSortableListItemProps<D extends Datum> {
    item: D;
    state: {
        isOver: boolean;
        isDragging: boolean;
    };
    rootProps: {
        ref: RefCallback<HTMLElement>;
        style: Pick<CSSProperties, 'transform' | 'transition'>;
    };
    dragHandleProps: DraggableAttributes & SyntheticListenerMap;
}

export type Item<D extends Datum> = {
    data: D;
    render(props: RenderSortableListItemProps<D>): ReactElement;
};
function Item<D extends Datum>({ data, render }: Item<D>) {
    const { attributes, listeners, setNodeRef, transform, transition, isOver, isDragging } = useSortable({
        id: data.id,
        transition: { duration: 150, easing: 'cubic-bezier(0.4, 0, 0.2, 1)' },
    });
    const $transform = transform
        ? CSS.Transform.toString({ x: transform.x, y: transform.y, scaleX: 1, scaleY: 1 })
        : undefined;

    type RenderProps = RenderSortableListItemProps<D>;
    return render({
        item: data,
        state: { isOver, isDragging },
        rootProps: {
            ref: setNodeRef,
            style: { transform: $transform, transition },
        },
        dragHandleProps: {
            ...attributes,
            ...listeners,
        },
    } as RenderProps);
}

//
// Wrapper
//

export interface SortableProps<D extends Datum> {
    items: Array<D>;
    onChange(items: Array<D>): void;
    renderItem(props: RenderSortableListItemProps<D>): ReactElement;

    className?: string;
    style?: CSSProperties;
}
export function Sortable<D extends Datum>(props: SortableProps<D>) {
    const { items, renderItem, onChange, className, ...rest } = props;

    const sensors = useSensors(
        useSensor(PointerSensor),
        useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
    );

    function handleDragEnd(e: DragEndEvent) {
        if (e.active.id === e.over?.id) return;

        const oldIndex = items.findIndex(x => x.id === e.active.id);
        const newIndex = items.findIndex(x => x.id === e.over?.id);
        const updated = arrayMove(items, oldIndex, newIndex);

        onChange(updated);
    }
    return (
        <div {...rest} className={cn(css.root, className)}>
            <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
                <SortableContext
                    items={items}
                    strategy={verticalListSortingStrategy}
                    children={items.map(d => <Item data={d} key={d.id} render={renderItem} />)}
                />
            </DndContext>
        </div>
    );
}
