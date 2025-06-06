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
