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
import type { IntlShape } from 'react-intl';

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

function requiredError(intl: IntlShape): string {
    return intl.formatMessage({
        defaultMessage: 'Value is required',
        description: 'Error under a widget setting left empty',
    });
}

function notNumberError(intl: IntlShape): string {
    return intl.formatMessage({
        defaultMessage: 'Not a number',
        description: 'Error under a numeric widget setting holding something other than a number',
    });
}

// The wire field is `int32`; protobuf's encoder throws on anything past it.
const INT32_MIN = -2_147_483_648;
const INT32_MAX = 2_147_483_647;

// The server's `MAX_PARAM_STRING_LENGTH`, which counts UTF-8 bytes rather than characters.
const MAX_STRING_BYTES = 1024;
const utf8 = new TextEncoder();

export function parseFormifiedValue(intl: IntlShape, def: pb.ManifestParamDefinition, raw: FieldValue): ParseResult {
    if (def.kind.case === 'paramArray') {
        invariant(Array.isArray(raw), `list param "${def.key}" holds a non-list value`);
        return parseList(
            intl,
            def.kind.value,
            raw.map(row => row.value),
        );
    }
    invariant(!Array.isArray(raw), `scalar param "${def.key}" holds a list value`);
    return parseScalar(intl, def.kind, raw, def.isOptional);
}

function parseScalar(intl: IntlShape, kind: ScalarKind, raw: ScalarValue, isOptional: boolean): ScalarParseResult {
    switch (kind.case) {
        case 'paramString': {
            if (raw === null || raw === '') {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: requiredError(intl) };
            }
            if (typeof raw !== 'string') return { ok: false, error: requiredError(intl) };
            if (utf8.encode(raw).length > MAX_STRING_BYTES)
                return {
                    ok: false,
                    error: intl.formatMessage(
                        {
                            defaultMessage: 'Must be at most {max} bytes',
                            description: 'Error under a widget text setting too long to store; max is always 1024',
                        },
                        { max: MAX_STRING_BYTES },
                    ),
                };

            const error = lengthError(intl, raw, kind.value);
            if (error) return { ok: false, error };
            return { ok: true, value: stringValue(raw) };
        }
        case 'paramTimezone': {
            if (raw === null || raw === '') {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: requiredError(intl) };
            }
            if (typeof raw !== 'string') return { ok: false, error: requiredError(intl) };
            return { ok: true, value: stringValue(raw) };
        }
        case 'paramInteger':
        case 'paramDouble': {
            const wantInt = kind.case === 'paramInteger';
            const inner = kind.value;
            if (raw === null || (typeof raw === 'string' && raw.trim() === '')) {
                if (isOptional) return { ok: true, value: nullValue() };
                return { ok: false, error: requiredError(intl) };
            }
            if (typeof raw !== 'string') return { ok: false, error: notNumberError(intl) };
            const n = Number(raw.trim());
            if (!Number.isFinite(n)) return { ok: false, error: notNumberError(intl) };
            if (wantInt && !Number.isInteger(n))
                return {
                    ok: false,
                    error: intl.formatMessage({
                        defaultMessage: 'Not an integer',
                        description: 'Error under a whole-number widget setting holding a fraction',
                    }),
                };
            const min = inner.min ?? (wantInt ? INT32_MIN : undefined);
            const max = inner.max ?? (wantInt ? INT32_MAX : undefined);
            if (min !== undefined && n < min)
                return {
                    ok: false,
                    error: intl.formatMessage(
                        {
                            defaultMessage: 'Must be at least {min}',
                            description: 'Error under a numeric widget setting below its lowest value; min is a number',
                        },
                        { min },
                    ),
                };
            if (max !== undefined && n > max)
                return {
                    ok: false,
                    error: intl.formatMessage(
                        {
                            defaultMessage: 'Must be at most {max}',
                            description:
                                'Error under a numeric widget setting above its highest value; max is a number',
                        },
                        { max },
                    ),
                };
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

// Counts code points as the server does; `.length` counts UTF-16 units, an emoji as two.
function lengthError(intl: IntlShape, s: string, { minLength, maxLength }: pb.ParamString): string | undefined {
    const length = [...s].length;
    if (minLength !== undefined && length < minLength)
        return intl.formatMessage(
            {
                defaultMessage:
                    '{count, plural, one {Must be at least {count} character} other {Must be at least {count} characters}}',
                description: 'Error under a widget text setting shorter than allowed; count is its fewest characters',
            },
            { count: minLength },
        );
    if (maxLength !== undefined && length > maxLength)
        return intl.formatMessage(
            {
                defaultMessage:
                    '{count, plural, one {Must be at most {count} character} other {Must be at most {count} characters}}',
                description: 'Error under a widget text setting longer than allowed; count is its most characters',
            },
            { count: maxLength },
        );
    return undefined;
}

function countError(intl: IntlShape, count: number, array: pb.ParamArray): string | undefined {
    if (count < array.minItems)
        return intl.formatMessage(
            {
                defaultMessage:
                    '{count, plural, one {Must have at least {count} item} other {Must have at least {count} items}}',
                description: 'Error under a widget list setting with too few rows; count is its fewest rows',
            },
            { count: array.minItems },
        );
    if (count > array.maxItems)
        return intl.formatMessage(
            {
                defaultMessage:
                    '{count, plural, one {Must have at most {count} item} other {Must have at most {count} items}}',
                description: 'Error under a widget list setting with too many rows; count is its most rows',
            },
            { count: array.maxItems },
        );
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
function repeatErrors(intl: IntlShape, array: pb.ParamArray, values: pb.FieldValue[]): Array<RowError | undefined> {
    const { uniqueItems } = array;

    const keys = uniqueItems.case === 'by' ? uniqueItems.value.keys : undefined;
    const identities = values.map(value => (keys ? keyFields(value, keys) : [value]));

    return identities.map((identity, i) => {
        const first = identities.findIndex(earlier => sameIdentity(earlier, identity));
        if (first === i) return undefined;
        const position = first + 1;
        if (!keys)
            return {
                error: intl.formatMessage(
                    {
                        defaultMessage: 'Repeats item {position}',
                        description:
                            'Error on a row of a widget list setting that must not repeat; position is the number of the earlier equal row',
                    },
                    { position },
                ),
            };
        const error = intl.formatMessage(
            {
                defaultMessage: 'Same as item {position}',
                description:
                    'Error on a row of a widget list setting whose identifying fields repeat; position is the number of the earlier row',
            },
            { position },
        );
        const [only, ...more] = keys;
        if (only !== undefined && more.length === 0) return { fields: { [only]: error } };
        return { error, markedFields: keys };
    });
}

function parseList(intl: IntlShape, array: pb.ParamArray, raw: Array<ListItem['value']>): ParseResult {
    const error = countError(intl, raw.length, array);
    // As on the server: past the bound, only the count is reported.
    if (raw.length > array.maxItems) return { ok: false, error };

    const kind = itemKind(array);
    const parsed = raw.map(value => parseItem(intl, kind, value));
    const values = parsed.flatMap(r => (r.ok ? [r.value] : []));

    let items = parsed.map(r => (r.ok ? undefined : r.error));
    if (array.uniqueItems.case !== undefined && values.length === parsed.length) {
        items = repeatErrors(intl, array, values);
    }

    if (error || items.some(Boolean)) return { ok: false, error, items };

    return {
        ok: true,
        value: listValue(values),
    };
}

function parseItem(intl: IntlShape, kind: pb.ArrayItemKind['kind'], raw: ListItem['value']): ItemParseResult {
    if (kind.case === 'paramObject') {
        invariant(isObjectValue(raw), 'an object list row holds a scalar');
        return parseObject(intl, kind.value, raw);
    }
    invariant(!isObjectValue(raw), 'a scalar list row holds an object');
    const r = parseScalar(intl, kind, raw, false);
    return r.ok ? r : { ok: false, error: { error: r.error } };
}

function parseObject(intl: IntlShape, object: pb.ParamObject, raw: ObjectValue): ItemParseResult {
    const fields: Record<string, pb.FieldValue> = {};
    const errors: Record<string, string> = {};
    for (const field of object.fields) {
        const r = parseScalar(intl, field.kind, ownValue(raw, field.key) ?? null, field.isOptional);
        if (r.ok) fields[field.key] = r.value;
        else errors[field.key] = r.error;
    }
    if (Object.keys(errors).length > 0) return { ok: false, error: { fields: errors } };
    return { ok: true, value: objectValue(fields) };
}
