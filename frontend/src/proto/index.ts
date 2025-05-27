// Global utils
import { abort } from '@/lib/dom';
import { invert, camelCase } from 'es-toolkit';
import { type iFormErrors, hasFormErrors } from '@/lib/form';

// 3rd parties
import { create, type Message } from '@bufbuild/protobuf';
import { ConnectError, Code } from '@connectrpc/connect';
import * as WktError from './gen/google/rpc/error_details_pb';
import type { Timezone } from './pb';

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
export function parseFormErrors<FieldName extends keyof any>(
    error: RpcStatus,
    knownFieldNames: FieldName[],
): FormErrors<FieldName> {
    const res = {
        global: [error.message] as string[],
        fields: {} as Partial<Record<FieldName, null | string[]>>,
    } satisfies FormErrors<FieldName>;

    error.fieldViolations.forEach(x => {
        // Typescript codegen transforms proto message fields to `camelCase`,
        // but backend will send them in their original `snake_case` form.
        const key = camelCase(x.field) as FieldName;

        if (knownFieldNames.includes(key)) {
            if (Array.isArray(res.fields[key])) res.fields[key].push(x.description);
            else res.fields[key] = [x.description];
        } else {
            res.global.push(x.description);
        }
    });

    return res;
}
export function renderFieldErrorsAsList(fieldErrors: Maybe<string[]>): null | string {
    if (!fieldErrors) return null;
    if (fieldErrors.length === 1) return fieldErrors[0];
    return fieldErrors.map(x => `- ${x}`).join('\n');
}
export function collectAllErrors(error: unknown | Error | ConnectError): null | string[] {
    const $ = parseFormErrors(parseError(error), []);
    return $.global ?? null;
}
export function collectAllErrorsAsFormattedList(error: unknown | Error | ConnectError): null | string {
    const $ = collectAllErrors(error);
    return renderFieldErrorsAsList($);
}

export type MessageFields<T extends Message> = keyof Omit<T, '$unknown' | '$typeName'>;
export type FormValues<T extends Message> = { [Key in MessageFields<T>]: T[Key] };
export type FormState<T extends Message, ExtraValues extends void | Record<string, any> = void> = {
    values: ExtraValues extends void ? FormValues<T> : FormValues<T & ExtraValues>;
    errors: ExtraValues extends void ? FormErrors<MessageFields<T>> : FormErrors<MessageFields<T & ExtraValues>>;
};

export function renderTimezone(tz: Maybe<Timezone>): string {
    if (!tz) return 'N/A';
    return `UTC${tz.offset} (${tz.label})`;
}

// Utilities index
export * from './pb';
export * from './rpc';

export {
    abort,
    create,
    ConnectError,
    hasFormErrors,
    // Types
    type Message,
};
