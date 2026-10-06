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

import * as pb from '@/proto';
import { assertUnreachable, ownValue } from '@/lib/ts';
import type { FieldValue, ListItem, ObjectValue, RowError, ScalarKind, ScalarValue } from './value';

export type ParseFailure = { ok: false; error?: string; items?: Array<RowError | undefined> };
export type ParseResult = { ok: true; value: pb.FieldValue } | ParseFailure;
type ScalarParseResult = { ok: true; value: pb.FieldValue } | { ok: false; error: string };
type ItemParseResult = { ok: true; value: pb.FieldValue } | { ok: false; error: RowError };

function nullValue(): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, {
        kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
    });
}
function stringValue(v: string): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, { kind: { case: 'stringValue', value: v } });
}
function integerValue(n: number): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, { kind: { case: 'integerValue', value: n } });
}
function doubleValue(n: number): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, { kind: { case: 'doubleValue', value: n } });
}
function booleanValue(b: boolean): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, { kind: { case: 'booleanValue', value: b } });
}
function listValue(items: pb.FieldValue[]): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, {
        kind: { case: 'listValue', value: pb.create(pb.FieldValueListSchema, { items }) },
    });
}
function objectValue(fields: Record<string, pb.FieldValue>): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, {
        kind: { case: 'objectValue', value: pb.create(pb.FieldValuesSchema, { fields }) },
    });
}

export function itemKind(array: pb.ParamArray): pb.ArrayItemKind['kind'] {
    return array.items?.kind ?? { case: undefined };
}

function isObjectValue(v: ListItem['value']): v is ObjectValue {
    return typeof v === 'object' && v !== null;
}

const ERR_REQUIRED = 'Value is required';
const ERR_NOT_NUMBER = 'Not a number';
const ERR_NOT_INTEGER = 'Not an integer';

// The wire field is `int32`; protobuf's encoder throws on anything past it.
const INT32_MIN = -2_147_483_648;
const INT32_MAX = 2_147_483_647;

// The server's `MAX_PARAM_STRING_LENGTH`, which counts UTF-8 bytes rather than characters.
const MAX_STRING_BYTES = 1024;
const utf8 = new TextEncoder();

export function parseFormifiedValue(def: pb.ManifestParamDefinition, raw: FieldValue): ParseResult {
    if (def.kind.case === 'paramArray') {
        invariant(Array.isArray(raw), `list param "${def.key}" holds a non-list value`);
        return parseList(
            def.kind.value,
            raw.map(row => row.value),
        );
    }
    invariant(!Array.isArray(raw), `scalar param "${def.key}" holds a list value`);
    return parseScalar(def.kind, raw, def.isOptional);
}

function parseScalar(kind: ScalarKind, raw: ScalarValue, isOptional: boolean): ScalarParseResult {
    switch (kind.case) {
        case 'paramString': {
            if (raw === null || raw === '') {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: ERR_REQUIRED };
            }
            if (typeof raw !== 'string') return { ok: false, error: ERR_REQUIRED };
            if (utf8.encode(raw).length > MAX_STRING_BYTES)
                return { ok: false, error: `Must be at most ${MAX_STRING_BYTES} bytes` };

            const error = lengthError(raw, kind.value);
            if (error) return { ok: false, error };
            return { ok: true, value: stringValue(raw) };
        }
        case 'paramTimezone': {
            if (raw === null || raw === '') {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: ERR_REQUIRED };
            }
            if (typeof raw !== 'string') return { ok: false, error: ERR_REQUIRED };
            return { ok: true, value: stringValue(raw) };
        }
        case 'paramInteger':
        case 'paramDouble': {
            const wantInt = kind.case === 'paramInteger';
            const inner = kind.value;
            if (raw === null || (typeof raw === 'string' && raw.trim() === '')) {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: ERR_REQUIRED };
            }
            if (typeof raw !== 'string') return { ok: false, error: ERR_NOT_NUMBER };
            const n = Number(raw.trim());
            if (!Number.isFinite(n)) return { ok: false, error: ERR_NOT_NUMBER };
            if (wantInt && !Number.isInteger(n)) return { ok: false, error: ERR_NOT_INTEGER };
            const min = inner.min ?? (wantInt ? INT32_MIN : undefined);
            const max = inner.max ?? (wantInt ? INT32_MAX : undefined);
            if (min !== undefined && n < min) return { ok: false, error: `Must be at least ${min}` };
            if (max !== undefined && n > max) return { ok: false, error: `Must be at most ${max}` };
            return { ok: true, value: wantInt ? integerValue(n) : doubleValue(n) };
        }
        case 'paramBoolean':
            return { ok: true, value: booleanValue(raw === true) };
        case undefined:
            return { ok: true, value: nullValue() };
        default:
            return assertUnreachable(kind, 'scalar param kind');
    }
}

function characterCount(n: number): string {
    return n === 1 ? '1 character' : `${n} characters`;
}

// Counts code points as the server does; `.length` counts UTF-16 units, an emoji as two.
function lengthError(s: string, { minLength, maxLength }: pb.ParamString): string | undefined {
    const length = [...s].length;
    if (minLength !== undefined && length < minLength) return `Must be at least ${characterCount(minLength)}`;
    if (maxLength !== undefined && length > maxLength) return `Must be at most ${characterCount(maxLength)}`;
    return undefined;
}

function itemCount(n: number): string {
    return n === 1 ? '1 item' : `${n} items`;
}

function countError(count: number, array: pb.ParamArray): string | undefined {
    if (count < array.minItems) return `Must have at least ${itemCount(array.minItems)}`;
    if (count > array.maxItems) return `Must have at most ${itemCount(array.maxItems)}`;
    return undefined;
}

function keyFields(row: pb.FieldValue, keys: string[]): pb.FieldValue[] {
    const { kind } = row;
    invariant(kind.case === 'objectValue', 'an object list row parses to an object');
    return keys.map(key => {
        const value = ownValue(kind.value.fields, key);
        invariant(value, `a parsed row holds its key field "${key}"`);
        return value;
    });
}

function sameIdentity(a: pb.FieldValue[], b: pb.FieldValue[]): boolean {
    return a.every((value, i) => {
        const other = b[i];
        return other !== undefined && pb.equals(pb.FieldValueSchema, value, other);
    });
}

// A single repeated key is reported at its field;
// a whole or composite repeat once at the row, with the fields it covers marked.
function repeatErrors(array: pb.ParamArray, values: pb.FieldValue[]): Array<RowError | undefined> {
    const { uniqueItems } = array;

    const keys = uniqueItems.case === 'by' ? uniqueItems.value.keys : undefined;
    const identities = values.map(value => (keys ? keyFields(value, keys) : [value]));

    return identities.map((identity, i) => {
        const first = identities.findIndex(earlier => sameIdentity(earlier, identity));
        if (first === i) return undefined;
        if (!keys) return { error: `Repeats item ${first + 1}` };
        const error = `Same as item ${first + 1}`;
        const [only, ...more] = keys;
        if (only !== undefined && more.length === 0) return { fields: { [only]: error } };
        return { error, markedFields: keys };
    });
}

function parseList(array: pb.ParamArray, raw: Array<ListItem['value']>): ParseResult {
    const error = countError(raw.length, array);
    // As on the server: past the bound, only the count is reported.
    if (raw.length > array.maxItems) return { ok: false, error };

    const kind = itemKind(array);
    const parsed = raw.map(value => parseItem(kind, value));
    const values = parsed.flatMap(r => (r.ok ? [r.value] : []));

    let items = parsed.map(r => (r.ok ? undefined : r.error));
    if (array.uniqueItems.case !== undefined && values.length === parsed.length) items = repeatErrors(array, values);

    if (error || items.some(Boolean)) return { ok: false, error, items };

    return {
        ok: true,
        value: listValue(values),
    };
}

function parseItem(kind: pb.ArrayItemKind['kind'], raw: ListItem['value']): ItemParseResult {
    if (kind.case === 'paramObject') {
        invariant(isObjectValue(raw), 'an object list row holds a scalar');
        return parseObject(kind.value, raw);
    }
    invariant(!isObjectValue(raw), 'a scalar list row holds an object');
    const r = parseScalar(kind, raw, false);
    return r.ok ? r : { ok: false, error: { error: r.error } };
}

function parseObject(object: pb.ParamObject, raw: ObjectValue): ItemParseResult {
    const fields: Record<string, pb.FieldValue> = {};
    const errors: Record<string, string> = {};
    for (const field of object.fields) {
        const r = parseScalar(field.kind, ownValue(raw, field.key) ?? null, field.isOptional);
        if (r.ok) fields[field.key] = r.value;
        else errors[field.key] = r.error;
    }
    if (Object.keys(errors).length > 0) return { ok: false, error: { fields: errors } };
    return { ok: true, value: objectValue(fields) };
}
