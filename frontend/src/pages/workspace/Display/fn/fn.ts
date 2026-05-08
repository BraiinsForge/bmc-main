import invariant from 'invariant';
import { cloneDeep } from 'es-toolkit';

import * as pb from '@/proto';
import { assertUnreachable } from '@/lib/ts';

import * as C from './const';
import type { WidgetOrPlaceholder, WidgetsOccupandyMap, WidgetsWithPlaceholders } from './const';

/**
 * To allow the user to:
 *  - add new widgets where there are none
 *  - move existing widget to an empty space
 * we have inject placeholders into the widgets array.
 */
export function mapOccupiedSlots<T extends C.Located>(input: T[]): WidgetsOccupandyMap {
    const res = cloneDeep(C.EMPTY_WIDGETS_OCCUPANDY_MAP) as WidgetsOccupandyMap;

    input.forEach(({ position, size }) => {
        invariant(size, 'size is required');
        invariant(position, 'position is required');

        const { row, col } = position;
        switch (size) {
            // 1×1
            case pb.WidgetSize.SMALL: {
                res[row][col] = true;
                break;
            }

            // 1×2
            case pb.WidgetSize.MEDIUM: {
                res[row].splice(col, 2, true, true);
                break;
            }

            // 2×2
            case pb.WidgetSize.LARGE: {
                res[row].splice(col, 2, true, true);
                res[row + 1].splice(col, 2, true, true);
                break;
            }

            // 2×4
            case pb.WidgetSize.FULL: {
                res[row].splice(col, 4, true, true, true, true);
                res[row + 1].splice(col, 4, true, true, true, true);
                break;
            }

            default:
                assertUnreachable(size, 'widget size');
        }
    });

    return res;
}

export function injectPlaceholdersToUnoccupiedSlots(input: pb.Widget[]): WidgetsWithPlaceholders {
    const res: WidgetsWithPlaceholders = [...input];

    const map = mapOccupiedSlots(input);
    map.forEach((row: boolean[], rowInd: number) => {
        row.forEach((occupied: boolean, colInd: number) => {
            if (occupied) return;
            res.push(C.placeholder(rowInd, colInd));
        });
    });

    return res;
}

/**
 * Placeholder are just our local presentational utility that backend does not care about.
 * This means that we have to get rid of them before sending widgets to the backend.
 */
export function removePlaceholders(input: WidgetsWithPlaceholders): pb.Widget[] {
    return input.filter((x: WidgetOrPlaceholder): x is pb.Widget => {
        return !('placeholder' in x) || x.placeholder === false;
    });
}

export function fitsRightDown(map: WidgetsOccupandyMap, position: pb.WidgetPosition, size: C.Size): C.MaybePosition {
    const span = C.WIDGET_SIZE_TO_SPAN[size];

    const slice: boolean[] = map
        // Vertical slice
        .slice(position.row, position.row + span.rows)
        // Horizontal slice + flatten for easier emptiness check
        .flatMap(row => row.slice(position.col, position.col + span.cols));

    const fits =
        // Slice only makes sense as a target when its big enough
        slice.length === span.rows * span.cols &&
        // and all slots in it are empty (false means empty)
        slice.every(slot => !slot);

    return fits ? position : null;
}
export function fitsRightAbove(map: WidgetsOccupandyMap, position: pb.WidgetPosition, size: C.Size): C.MaybePosition {
    const span = C.WIDGET_SIZE_TO_SPAN[size];

    // For right-up, we go right from pos.col and up from pos.row
    const startRow = position.row - span.rows + 1;
    if (startRow < 0) return null;

    const slice: boolean[] = map
        // Vertical slice (upward from pos.row)
        .slice(startRow, position.row + 1)
        // Horizontal slice (rightward from pos.col)
        .flatMap(row => row.slice(position.col, position.col + span.cols));

    const fits = slice.length === span.rows * span.cols && slice.every(slot => !slot);
    return fits ? C.pos(startRow, position.col) : null;
}
export function fitsLeftDown(map: WidgetsOccupandyMap, position: pb.WidgetPosition, size: C.Size): C.MaybePosition {
    const span = C.WIDGET_SIZE_TO_SPAN[size];

    // For left-down, we go left from pos.col and down from pos.row
    const startCol = position.col - span.cols + 1;
    if (startCol < 0) return null;

    const slice: boolean[] = map
        // Vertical slice (downward from pos.row)
        .slice(position.row, position.row + span.rows)
        // Horizontal slice (leftward from pos.col)
        .flatMap(row => row.slice(startCol, position.col + 1));

    const fits = slice.length === span.rows * span.cols && slice.every(slot => !slot);
    return fits ? C.pos(position.row, startCol) : null;
}
export function fitsLeftAbove(map: WidgetsOccupandyMap, position: pb.WidgetPosition, size: C.Size): C.MaybePosition {
    const span = C.WIDGET_SIZE_TO_SPAN[size];

    // For left-up, we go left from pos.col and up from pos.row
    const startRow = position.row - span.rows + 1;
    const startCol = position.col - span.cols + 1;
    if (startRow < 0 || startCol < 0) return null;

    const slice: boolean[] = map
        // Vertical slice (upward from pos.row)
        .slice(startRow, position.row + 1)
        // Horizontal slice (leftward from pos.col)
        .flatMap(row => row.slice(startCol, position.col + 1));

    const fits = slice.length === span.rows * span.cols && slice.every(slot => !slot);
    return fits ? C.pos(startRow, startCol) : null;
}
function fitsAround(map: WidgetsOccupandyMap, position: pb.WidgetPosition, size: C.Size): C.MaybePosition {
    return (
        fitsRightDown(map, position, size) ||
        fitsRightAbove(map, position, size) ||
        fitsLeftDown(map, position, size) ||
        fitsLeftAbove(map, position, size)
    );
}

/**
 * Validates whether provided widget can be added to the pool.
 * The "widget" position must already be the target one, quite obviously.
 *
 * ---
 *
 * Some notes on the algorithm used:
 *
 * Since we don't require users to always drop the widget into a top-left corner
 * of space where it fits, we need to check few additional slots to see if it fits
 * when this shift is accounted for.
 *
 * Now, since the backend does care about the top-left position being used
 * (and otherwise refuses to save the change if there is not enought space)
 * we have to report the canonical position back to the caller.
 */
export function getWidgetInsertionSlot(pool: C.Located[], widget: C.Located): null | pb.WidgetPosition {
    const pos: Maybe<pb.WidgetPosition> = cloneDeep(widget.position);
    invariant(pos, 'position is required');

    const size = widget.size;
    invariant(size, 'size is required');

    // Cleanup the input to make sure the widget being checked
    // is not still present in the pool… its place should be free now
    const cleanPool: C.Located[] = pool.filter(x => x.id !== widget.id);
    const map = mapOccupiedSlots(cleanPool);

    // No sense in trying when the position itself is occupied or out of bounds
    if (pos.row >= 0 && pos.col >= 0 && map[pos.row][pos.col] === true) return null;

    // We have to check for available space all around
    // the target position in this order:
    //  ↘ right-down
    //  ↗ right-up
    //  ↙ left-down
    //  ↖ left-up
    return fitsAround(map, pos, size);
}

/**
 * Take a widget of given position and size
 * and produce a list of placeholder widgets
 * of SMALL size that cover the same area.
 */
export function explodeWidgetIntoAtoms({ position, size }: C.Located): C.WidgetPlaceholder[] {
    invariant(position, 'position is required');
    const { row, col } = position;

    switch (size) {
        case pb.WidgetSize.UNSPECIFIED:
            throw new Error('invalid size');

        // 1×1
        case pb.WidgetSize.SMALL:
            return [C.placeholder(row, col)];

        // 1×2
        case pb.WidgetSize.MEDIUM:
            return [C.placeholder(row, col), C.placeholder(row, col + 1)];

        // 2×2
        case pb.WidgetSize.LARGE:
            return [
                // Top
                C.placeholder(row, col),
                C.placeholder(row, col + 1),
                // Bottom
                C.placeholder(row + 1, col),
                C.placeholder(row + 1, col + 1),
            ];

        // 2×4
        case pb.WidgetSize.FULL:
            return [
                // Top
                C.placeholder(row, col),
                C.placeholder(row, col + 1),
                C.placeholder(row, col + 2),
                C.placeholder(row, col + 3),
                // Bottom
                C.placeholder(row + 1, col),
                C.placeholder(row + 1, col + 1),
                C.placeholder(row + 1, col + 2),
                C.placeholder(row + 1, col + 3),
            ];
    }
}

/**
 * Given a pool of widgets and a widget to be moved, calculate valid drop slots.
 *
 * This is the same logic as when checking whether a drop zone is a valid target,
 * but here we iterate through all empty slots instead of checking a specific one.
 */
export function getValidDropSlots(pool: C.Located[], widget: C.Located): C.ValidDropSlots {
    const res: C.ValidDropSlots = new Set();

    const { position, size } = widget;
    invariant(position, 'widget.position is required');
    invariant(size, 'widget.size is required');

    // Cleanup the input to make sure the widget being checked
    // is not still present in the pool… its place should be free now
    const cleanPool: C.Located[] = pool.filter(x => x.id !== widget.id);
    const map = mapOccupiedSlots(cleanPool);

    map.forEach((row: boolean[], rowInd: number) => {
        row.forEach((occupied: boolean, colInd: number) => {
            if (occupied) return;

            const insertionSlot = fitsAround(map, C.pos(rowInd, colInd), size);
            if (insertionSlot) res.add(C.dropSlotKey(rowInd, colInd));
        });
    });

    return res;
}

/**
 * Project a WidgetDataStruct from the wire into the modal's
 * Record<string, WidgetDataValue> form state.
 */
export function widgetParamsToFormState(params: pb.WidgetDataStruct | undefined): Record<string, pb.WidgetDataValue> {
    if (!params) return {};
    const result: Record<string, pb.WidgetDataValue> = {};
    for (const [k, v] of Object.entries(params.fields)) {
        result[k] = pb.create(pb.WidgetDataValueSchema, v);
    }
    return result;
}

function makeNullValue(): pb.WidgetDataValue {
    return pb.create(pb.WidgetDataValueSchema, {
        kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
    });
}

function defaultParamValue(def: pb.ManifestParamDefinition): pb.WidgetDataValue {
    switch (def.kind.case) {
        case 'paramString':
            return pb.create(pb.WidgetDataValueSchema, {
                kind: { case: 'stringValue', value: def.kind.value.defaultValue ?? '' },
            });
        case 'paramTimezone':
            return def.kind.value.defaultValue !== undefined
                ? pb.create(pb.WidgetDataValueSchema, {
                      kind: { case: 'stringValue', value: def.kind.value.defaultValue },
                  })
                : makeNullValue();
        case 'paramInteger':
            return def.kind.value.defaultValue !== undefined
                ? pb.create(pb.WidgetDataValueSchema, {
                      kind: { case: 'integerValue', value: def.kind.value.defaultValue },
                  })
                : makeNullValue();
        case 'paramDouble':
            return def.kind.value.defaultValue !== undefined
                ? pb.create(pb.WidgetDataValueSchema, {
                      kind: { case: 'doubleValue', value: def.kind.value.defaultValue },
                  })
                : makeNullValue();
        case 'paramBoolean':
            return pb.create(pb.WidgetDataValueSchema, {
                kind: { case: 'booleanValue', value: def.kind.value.defaultValue ?? false },
            });
        default:
            return pb.create(pb.WidgetDataValueSchema, { kind: { case: 'stringValue', value: '' } });
    }
}

/**
 * Inverse of widgetParamsToFormState. Iterates the manifest (not the form
 * state) so unknown form keys cannot leak onto the wire.
 * Missing keys are materialized from manifest defaults so UpdateWidget
 * requests always carry a complete param map.
 */
export function formStateToWidgetDataStruct(
    manifest: pb.WidgetManifest,
    params: Record<string, pb.WidgetDataValue>,
): pb.WidgetDataStruct {
    const fields: Record<string, pb.WidgetDataValue> = {};
    for (const def of manifest.params) {
        fields[def.key] = params[def.key] ?? defaultParamValue(def);
    }
    return pb.create(pb.WidgetDataStructSchema, { fields });
}

/** Get available widget sizes for a given widget position */
export function getValidWidgetSizes(pool: C.Located[], slot: Pick<C.Located, 'id' | 'position'>): C.Size[] {
    invariant(slot.position, 'slot.position is required');

    const cleanPool = pool.filter(x => x.id !== slot.id);
    const map = mapOccupiedSlots(cleanPool);
    const { row, col } = slot.position;

    const res: C.Size[] = [];
    const sizes: C.Size[] = [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE];

    sizes.forEach(size => {
        const insertionSlot = fitsAround(map, C.pos(row, col), size);
        if (insertionSlot) res.push(size);
    });

    return res;
}
