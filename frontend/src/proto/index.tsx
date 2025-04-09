// Global utils
import { invert } from 'es-toolkit';
import { abort } from '@/lib/dom';
import type { iFormErrors } from '@/lib/form';
import type { PlainProtoMessage as Raw, PartialMessage } from '@/lib/proto';

// 3rd parties
import { create } from '@bufbuild/protobuf';
import { ConnectError, Code } from '@connectrpc/connect';
import * as WktError from './gen/google/rpc/error_details_pb';

export type RpcStatus = {
    rpc_reason_code: Code;
    rpc_reason_name: keyof typeof Code;
    message: null | string;

    fieldViolations: ReadonlyArray<WktError.BadRequest_FieldViolation>;
    requestInfo: ReadonlyArray<WktError.RequestInfo>;
};
const RPC_CODES_MAP = invert(Code) as { [K in Code]?: keyof typeof Code };

export function parseError(error: unknown | Error | ConnectError, defaultMessagePreffix?: string): RpcStatus {
    const connectError = ConnectError.from(error);

    const res: RpcStatus = {
        rpc_reason_name: RPC_CODES_MAP[connectError.code] || 'Unknown',
        rpc_reason_code: connectError.code || Code.Unknown,
        message: connectError.rawMessage || (error instanceof Error ? decodeURIComponent(error.message) : null),

        // https://connectrpc.com/docs/web/errors/#error-details
        fieldViolations: connectError.findDetails(WktError.BadRequestSchema).flatMap(x => x.fieldViolations),
        requestInfo: connectError.findDetails(WktError.RequestInfoSchema),
    };
    if (defaultMessagePreffix) res.message = `${defaultMessagePreffix}: ${res.message}`;

    return res;
}

// Given how the form errors are constructed and transmitted through gRPC,
// we can have multiple errors per field, so we need to account for that
// in our unified form errors type.
export type FormErrors<FieldName extends keyof any> = iFormErrors<FieldName, string[]>;
export function formErrorsAreEmpty<Errors extends FormErrors<any>>(errors: Maybe<Errors>): boolean {
    if (!errors) return true;
    if (errors.global?.length) return false;
    for (const field in Object.values(errors.fields || {})) {
        if (field?.length) return false;
    }
    return true;
}

export function parseFormErrors<FieldName extends string>(
    error: RpcStatus,
    knownFieldNames: FieldName[],
): FormErrors<FieldName> {
    const res = {
        global: [error.message] as string[],
        fields: {} as Record<FieldName, string[]>,
    } satisfies FormErrors<FieldName>;

    error.fieldViolations.forEach(x => {
        const field = x.field as FieldName;

        if (knownFieldNames.includes(field)) {
            if (Array.isArray(res.fields[field])) res.fields[field].push(x.description);
            else res.fields[field] = [x.description];
        } else {
            res.global.push(x.description);
        }
    });

    return res;
}

export function Value<T>(value: T): { value: T } {
    return { value };
}

// Utilities index
export * from './rpc';
export * from '@bufbuild/protobuf/wkt';

export {
    abort,
    create,
    ConnectError,
    // Types
    type Raw,
    type PartialMessage,
};

export type { CallOptions } from '@connectrpc/connect';
