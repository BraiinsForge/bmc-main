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

import type * as pb from '@/proto';
import { assertUnreachable } from '@/lib/ts';

/** The scalar-only oneof; a param's or a list item's scalar kind narrows to it. */
export type ScalarKind = pb.ObjectFieldDefinition['kind'];

export type ScalarValue = string | boolean | null;

export type ObjectValue = Record<string, ScalarValue>;

/** One row of a list field; `id` keeps the row's identity through reorders and removals. */
export interface ListItem {
    id: number;
    value: ScalarValue | ObjectValue;
}

export type FieldValue = ScalarValue | ListItem[];

export interface RowError {
    error?: string;
    fields?: Record<string, string | undefined>;
}

let lastListItemId = 0;

export function listItem(value: ListItem['value']): ListItem {
    lastListItemId += 1;
    return { id: lastListItemId, value };
}

export function defaultItemValue(kind: pb.ArrayItemKind['kind']): ListItem['value'] {
    if (kind.case !== 'paramObject') return defaultScalarValue(kind);
    return Object.fromEntries(kind.value.fields.map(field => [field.key, defaultScalarValue(field.kind)]));
}

export function defaultScalarValue(kind: ScalarKind): ScalarValue {
    switch (kind.case) {
        case 'paramString':
            return kind.value.defaultValue ?? '';
        case 'paramTimezone':
            return kind.value.defaultValue ?? null;
        case 'paramInteger':
        case 'paramDouble':
            return kind.value.defaultValue !== undefined ? String(kind.value.defaultValue) : null;
        case 'paramBoolean':
            return kind.value.defaultValue ?? false;
        case undefined:
            return null;
        default:
            return assertUnreachable(kind, 'scalar param kind');
    }
}
