// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { camelCase, get, set } from 'es-toolkit/compat';
import type { FieldBasedErrors } from './types';
import type { BadRequest_FieldViolation } from '../gen/google/rpc/error_details_pb';

type FieldViolation = BadRequest_FieldViolation;

/**
 * @example
 * parseFieldPath('emailAddresses[3].type[2]')
 * ['emailAddresses', 3, 'type', 2]
 */
export function parseFieldPath(path: string): string[] {
    return (
        path
            // » emailAddresses[3].type[2]
            // « emailAddresses[3.type[2
            .replaceAll(']', '')
            // » emailAddresses[3.type[2
            // « emailAddresses.3.type.2
            .replaceAll('[', '.')
            // » emailAddresses.3.type.2
            // « [emailAddresses, 3, type, 2]
            .split('.')
            .filter(Boolean)
    );
}

export function parseFieldViolations<Input extends Rec, KnownKey extends PropertyKey>(
    input: ReadonlyArray<FieldViolation>,
    knownKeys?: KnownKey[],
): { parsed: FieldBasedErrors<Input>; unmatched: string[] } {
    const unmatched: string[] = [];
    const parsed: FieldBasedErrors<Input> = {};

    input.forEach(f => {
        const fieldPath = f.field;
        const resultKey: string[] = parseFieldPath(fieldPath).map(camelCase);

        if (knownKeys && !knownKeys.includes(resultKey[0] as KnownKey)) {
            unmatched.push(f.description);
        } else {
            const cleanedValue = f.description
                // If the error message starts with the field path, we'll drop it
                .replace(fieldPath, '')
                // Drop empty quotes (single or double)
                .replace(/["']{2}/, '')
                // Drop trailing whitespace
                .trim();

            const value: string[] = [];
            const existingValue: Maybe<string[]> = get(parsed, resultKey);
            if (existingValue) value.push(...existingValue);
            value.push(cleanedValue);

            set(parsed, resultKey, value);
        }
    });

    return { parsed, unmatched };
}
