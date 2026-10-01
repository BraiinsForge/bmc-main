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

import invariant from 'invariant';
import { cloneDeep } from 'es-toolkit';
import { Code, ConnectError } from '@connectrpc/connect';
import type { IntlShape } from 'react-intl';

import * as pb from '@/proto';
import type { Capabilities } from '@/lib/system';
import { URLS } from '@/constants';
import { assertUnreachable, ownValue } from '@/lib/ts';
import {
    defaultScalarValue,
    listItem,
    type FieldValue,
    type ListItem,
    type RowError,
    type ScalarKind,
    type ScalarValue,
} from '@/components/ParamField/value';
import { itemKind, parseFormifiedValue, type ParseFailure } from '@/components/ParamField/parse';

import * as C from './const';
import type { WidgetOrPlaceholder, WidgetsOccupandyMap, WidgetsWithPlaceholders } from './const';

export function runningWidgetLimitErrorMessage(error: unknown, intl: IntlShape): null | string {
    if (ConnectError.from(error).code !== Code.ResourceExhausted) return null;
    return intl.formatMessage({ defaultMessage: 'Running widget limit reached.' });
}

// A hung write would hold up every write queued behind it,
// so each one gives up after this long.
//
// A resize answers only once the replaced widget has exited,
// which the server allows up to 10s (`GRACEFUL_SHUTDOWN_TIMEOUT`
// in `bmc/src/widget/manager.rs`); three times that means
// a dead connection rather than a slow device.
export const SCENE_WRITE_TIMEOUT_MS = 30_000;

/**
 * `error` as it was, unless it is a timeout,
 * which gets a message the page can show as it is.
 */
export function explainTimeout(error: unknown, intl: IntlShape): unknown {
    if (ConnectError.from(error).code !== Code.DeadlineExceeded) return error;
    const message = intl.formatMessage({ defaultMessage: "The device didn't answer in time." });
    return new ConnectError(message, Code.DeadlineExceeded);
}

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

// ---------------------------------------------------------------------------
// Formified params: raw user-input shape that mirrors the manifest's declared types.
// Numbers ride as raw strings (parsed at submit), booleans stay boolean,
// optional/empty as null, and a list param holds its rows.
// The formified→wire converter has no path to emit a type-mismatched payload —
// the parse-shape gate catches every failure mode the type system can't express.
// ---------------------------------------------------------------------------

export type FormifiedValue = FieldValue;
export type FormifiedParams = Record<string, FormifiedValue>;
export interface ParamsFormErrors {
    global: string[];
    fields: Record<string, string[] | undefined>;
    /** A list param's item violations, by item index. */
    items?: Record<string, Array<RowErrors | undefined> | undefined>;
    /** Binding violations, keyed by slot key. */
    credentials?: Record<string, string[]>;
}

export interface RowErrors {
    errors?: string[];
    fields?: Record<string, string[] | undefined>;
}

function hasRowErrors(row: RowErrors | undefined): boolean {
    return !!row?.errors?.length || Object.values(row?.fields ?? {}).some(errors => !!errors?.length);
}

export function hasItemErrors(errors: null | ParamsFormErrors): boolean {
    return Object.values(errors?.items ?? {}).some(rows => rows?.some(hasRowErrors));
}

/** Each row's first violation, and each field's, as a list field shows them. */
export function itemErrorsOf(errors: null | ParamsFormErrors, key: string): Array<RowError | undefined> | undefined {
    return ownValue(errors?.items, key)?.map(row => {
        if (!row) return undefined;
        const fields = row.fields && Object.entries(row.fields).map(([field, list]) => [field, list?.[0]]);
        return { error: row.errors?.[0], fields: fields && Object.fromEntries(fields) };
    });
}

export function clearFieldError(errors: null | ParamsFormErrors, key: string): null | ParamsFormErrors {
    if (!errors) return null;
    const hadFieldError = !!ownValue(errors.fields, key)?.length || !!ownValue(errors.items, key)?.some(hasRowErrors);
    return {
        global: hadFieldError ? [] : errors.global,
        fields: { ...errors.fields, [key]: undefined },
        items: { ...errors.items, [key]: undefined },
    };
}

function toRowErrors(row: RowError | undefined): RowErrors | undefined {
    if (!row) return undefined;
    const fields = row.fields && Object.entries(row.fields).map(([key, error]) => [key, error ? [error] : undefined]);
    return { errors: row.error ? [row.error] : undefined, fields: fields && Object.fromEntries(fields) };
}

function withFailure(errors: ParamsFormErrors, key: string, failure: ParseFailure): ParamsFormErrors {
    return {
        ...errors,
        fields: { ...errors.fields, [key]: failure.error ? [failure.error] : undefined },
        items: { ...errors.items, [key]: failure.items?.map(toRowErrors) },
    };
}

export function revalidateField(
    errors: null | ParamsFormErrors,
    def: pb.ManifestParamDefinition,
    value: FormifiedValue,
): null | ParamsFormErrors {
    const cleared = clearFieldError(errors, def.key);
    const r = parseFormifiedValue(def, value);
    if (r.ok) return cleared;
    return withFailure(cleared ?? { global: [], fields: {} }, def.key, r);
}

export function defaultFormifiedValue(def: pb.ManifestParamDefinition): FormifiedValue {
    if (def.kind.case === 'paramArray') {
        const kind = itemKind(def.kind.value);
        return def.kind.value.defaultValue.map(v => listItem(readWireItem(kind, v)));
    }
    return defaultScalarValue(def.kind);
}

function readWireItem(kind: pb.ArrayItemKind['kind'], v: pb.FieldValue): ListItem['value'] {
    if (kind.case !== 'paramObject') return readWireScalar(kind, v);
    const wire = v.kind.case === 'structValue' ? v.kind.value.fields : {};
    return Object.fromEntries(
        kind.value.fields.map(field => {
            const value = ownValue(wire, field.key);
            return [field.key, value ? readWireScalar(field.kind, value) : defaultScalarValue(field.kind)];
        }),
    );
}

function readWireScalar(kind: ScalarKind, v: pb.FieldValue): ScalarValue {
    if (v.kind.case === 'nullValue') {
        return kind.case === 'paramBoolean' ? false : null;
    }
    switch (kind.case) {
        case 'paramString':
        case 'paramTimezone':
            return v.kind.case === 'stringValue' ? v.kind.value : defaultScalarValue(kind);
        case 'paramInteger':
            return v.kind.case === 'integerValue' ? String(v.kind.value) : defaultScalarValue(kind);
        case 'paramDouble':
            return v.kind.case === 'doubleValue' ? String(v.kind.value) : defaultScalarValue(kind);
        case 'paramBoolean':
            return v.kind.case === 'booleanValue' ? v.kind.value : false;
        default:
            return defaultScalarValue(kind);
    }
}

function readWireAsFormified(def: pb.ManifestParamDefinition, v: pb.FieldValue): FormifiedValue {
    if (def.kind.case !== 'paramArray') return readWireScalar(def.kind, v);
    if (v.kind.case !== 'listValue') return defaultFormifiedValue(def);
    const kind = itemKind(def.kind.value);
    return v.kind.value.items.map(item => listItem(readWireItem(kind, item)));
}

export function widgetParamsToFormifiedState(
    manifest: pb.WidgetManifest,
    params: pb.FieldValues | undefined,
): FormifiedParams {
    const out: FormifiedParams = {};
    for (const def of manifest.params) {
        const wire = ownValue(params?.fields, def.key);
        out[def.key] = wire ? readWireAsFormified(def, wire) : defaultFormifiedValue(def);
    }
    return out;
}

export function buildFieldValues(
    manifest: pb.WidgetManifest,
    params: FormifiedParams,
): { ok: true; value: pb.FieldValues } | { ok: false; errors: ParamsFormErrors } {
    const fields: Record<string, pb.FieldValue> = {};
    let errors: null | ParamsFormErrors = null;
    for (const def of manifest.params) {
        const raw = Object.hasOwn(params, def.key) ? params[def.key] : defaultFormifiedValue(def);
        const r = parseFormifiedValue(def, raw);
        if (r.ok) fields[def.key] = r.value;
        else errors = withFailure(errors ?? { global: [], fields: {} }, def.key, r);
    }
    if (errors) return { ok: false, errors };
    return { ok: true, value: pb.create(pb.FieldValuesSchema, { fields }) };
}

/**
 * Stored params fitted to the manifest installed now, for a write that restores them.
 * A stored value is kept as sent, never re-encoded through the form.
 * The server refuses an update that names an undeclared key or leaves a declared one out,
 * so a key the manifest dropped is left out and one it added takes its default.
 */
export function fitStoredParams(manifest: pb.WidgetManifest, stored: pb.FieldValues): pb.FieldValues {
    const fields: Record<string, pb.FieldValue> = {};
    for (const def of manifest.params) {
        // A proto map is a plain object,
        // so a key like `toString` would otherwise read as inherited.
        if (Object.hasOwn(stored.fields, def.key)) {
            fields[def.key] = stored.fields[def.key];
            continue;
        }
        const fallback = parseFormifiedValue(def, defaultFormifiedValue(def));
        if (fallback.ok) fields[def.key] = fallback.value;
    }
    return pb.create(pb.FieldValuesSchema, { fields });
}

// A binding whose account is *gone* never arrives — `effective_bindings` drops it server-side.
// One whose account is the wrong type does: existence is all that filter checks,
// and a hand-edited config can mismatch a slot.
export function isMisbound(slot: pb.CredentialSlotDefinition, accounts: pb.Account[], boundAccountId: string): boolean {
    return !!boundAccountId && !accounts.some(a => a.id === boundAccountId && a.typeId === slot.typeId);
}

/** `bindings` less those naming an account that is gone. */
export function withoutDeletedAccounts(
    bindings: Record<string, string>,
    accounts: pb.Account[],
): Record<string, string> {
    return Object.fromEntries(
        Object.entries(bindings).filter(([, accountId]) => accounts.some(a => a.id === accountId)),
    );
}

function withoutRetiredSlots(bindings: Record<string, string>, manifest: pb.WidgetManifest): Record<string, string> {
    return Object.fromEntries(
        manifest.credentials
            .filter(slot => Object.hasOwn(bindings, slot.key))
            .map(slot => [slot.key, bindings[slot.key]]),
    );
}

/** Whether the server would accept `bindings` back: every key a declared slot, every account one that fits it. */
export function credentialBindingsValid(
    manifest: pb.WidgetManifest,
    accounts: pb.Account[],
    bindings: Record<string, string>,
): boolean {
    return Object.entries(bindings).every(([key, accountId]) => {
        const slot = manifest.credentials.find(s => s.key === key);
        return !!slot && !isMisbound(slot, accounts, accountId);
    });
}

export type CredentialBindingsWrite = 'preview' | 'cancel' | 'done';

/**
 * The bindings a dialog write carries, or `undefined`
 * to leave the server's own in place.
 *
 * None go out until the user changes one, so an edit that never
 * touched them is not refused for an account deleted since the page loaded.
 * Slots the manifest no longer declares are left out, the way dropped params are.
 *
 * A preview and its cancel also need the set the dialog opened with
 * to validate, since the cancel has to send that set back.
 */
export function credentialBindingsFor(
    write: CredentialBindingsWrite,
    session: {
        manifest: pb.WidgetManifest;
        accounts: pb.Account[];
        original: Record<string, string>;
        edited?: Record<string, string>;
    },
): undefined | { bindings: Record<string, string> } {
    const { manifest, accounts } = session;
    if (!session.edited) return undefined;
    const original = withoutRetiredSlots(session.original, manifest);
    const edited = withoutRetiredSlots(session.edited, manifest);
    if (write === 'done') return { bindings: edited };
    if (!credentialBindingsValid(manifest, accounts, original)) return undefined;
    return { bindings: write === 'cancel' ? original : edited };
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

export function combinedSceneAvailable(caps: Capabilities): boolean {
    return caps.combinedScenesSupported;
}

export function combinedEditorRedirectTarget(caps: Capabilities): null | string {
    return caps.combinedScenesSupported ? null : URLS.pages.display.list;
}
