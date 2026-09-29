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

import * as pb from '@/proto';
import { ownValue } from '@/lib/ts';
import type { ParamsFormErrors, RowErrors } from './fn';

const ITEM_INDEX = /^\d+$/;

function ownList<T>(record: Record<string, T[] | undefined>, key: string): T[] {
    const list = ownValue(record, key) ?? [];
    record[key] = list;
    return list;
}

/**
 * Not `parseFormErrors`: it nests paths into one object, where a param's own
 * error list and its items' index steps would land on the same array.
 */
export function mapManifestUpdateError(rawError: unknown): ParamsFormErrors {
    const { message, fieldViolations } = pb.parseError(rawError);
    const fields: Record<string, string[] | undefined> = {};
    const items: Record<string, Array<RowErrors | undefined> | undefined> = {};
    const credentials: Record<string, string[]> = {};
    const global = message ? [message] : [];

    for (const { field, description } of fieldViolations) {
        // Field names arrive camelCased:
        // `credential_bindings` → `credentialBindings`.
        const [root, key, index, rowField, ...rest] = pb.parseFieldPath(field);
        if (root === 'params' && key !== undefined && index === undefined) {
            ownList(fields, key).push(description);
        } else if (
            root === 'params' &&
            key !== undefined &&
            index !== undefined &&
            ITEM_INDEX.test(index) &&
            rest.length === 0
        ) {
            const row = (ownList(items, key)[Number(index)] ??= {});
            if (rowField === undefined) (row.errors ??= []).push(description);
            else ownList((row.fields ??= {}), rowField).push(description);
        } else if (root === 'credentialBindings' && key !== undefined && index === undefined) {
            ownList(credentials, key).push(description);
        } else {
            global.push(description);
        }
    }

    return { global, fields, items, credentials };
}
