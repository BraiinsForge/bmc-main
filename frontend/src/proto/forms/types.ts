import { invert } from 'es-toolkit';
import type { Primitive } from 'type-fest';

import { Code, type ConnectError } from '@connectrpc/connect';
import type { BadRequest_FieldViolation, RequestInfo } from '../gen/google/rpc/error_details_pb';

export type RpcStatus = {
    rpc_reason_code: Code;
    rpc_reason_name: keyof typeof Code;
    message: null | string;

    fieldViolations: ReadonlyArray<BadRequest_FieldViolation>;
    requestInfo: ReadonlyArray<RequestInfo>;
};
export const RPC_CODES_MAP = invert(Code) as { [K in Code]?: keyof typeof Code };

/**
 * Represents potentially unknown thrown error types comming out of the rpc calls.
 * This will in practice be caught by try/catch and passed in if not detected as abort error.
 */
export type BareException = unknown | Error | ConnectError;

/**
 * Each field will have it's errors represented as an array of strings.
 * This is because the way protobuf transfers them means there always
 * can be more then one occurance of any given field key.
 */
export type FieldErrors = string[];

/**
 * Based on a payload type passed into an RPC call, this will represent errors potentially comming back.
 *
 * @example
 * type UserInput = {
 *   name: string;
 *   age: number;
 *   addresses: Array<{
 *     street: string;
 *     city: string;
 *   }>;
 * };
 *
 * // Resulting type will be:
 * type UserErrors = {
 *   name?: string[];
 *   age?: string[];
 *   addresses?: Array<{
 *     street?: string[];
 *     city?: string[];
 *   }>;
 * };
 */
export type FieldBasedErrors<Input extends Rec> = {
    [K in keyof Input]?: Input[K] extends Primitive
        ? FieldErrors
        : Input[K] extends Primitive[]
          ? Array<FieldErrors>
          : Input[K] extends Array<Rec>
            ? Array<FieldBasedErrors<Input[K][number]>>
            : Input[K] extends Rec
              ? FieldBasedErrors<Input[K]>
              : never;
};

/**
 * Thin wrapper type over the above illustrated example
 * with addition of global errors array where we'll store
 * anything not recognized as declared known field
 * and the global error message.
 */
export type FormErrors<Input extends Rec> = {
    global: string[];
    fields: FieldBasedErrors<Input>;
};

export type MessageFields<T extends Rec> = keyof Omit<T, '$unknown' | '$typeName'>;
export type FormValues<T extends Rec> = { [Key in MessageFields<T>]: T[Key] };

export type FormState<Input extends Rec, ExtraValues extends void | Rec = void> = {
    values: ExtraValues extends void ? FormValues<Input> : FormValues<Input & ExtraValues>;
    errors: null | (ExtraValues extends void ? FormErrors<Input> : FormErrors<Input & ExtraValues>);
};
