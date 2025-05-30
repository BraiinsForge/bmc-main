import { ConnectError, Code } from '@connectrpc/connect';
import router from '@/routes';
import { URLS } from '@/constants';

import { hasFormErrors } from '@/lib/form';

import { RPC_CODES_MAP } from './types';
import type { BareException, RpcStatus, FieldBasedErrors, FormErrors, FieldErrors } from './types';

import { parseFieldViolations } from './parseFieldViolation';
import { BadRequestSchema, RequestInfoSchema } from '../gen/google/rpc/error_details_pb';

export function parseError(error: BareException, defaultMessagePreffix?: string): RpcStatus {
    const connectError = ConnectError.from(error);

    const res: RpcStatus = {
        rpc_reason_name: RPC_CODES_MAP[connectError.code] || 'Unknown',
        rpc_reason_code: connectError.code || Code.Unknown,
        message: connectError.rawMessage || (error instanceof Error ? decodeURIComponent(error.message) : null),

        // https://connectrpc.com/docs/web/errors/#error-details
        fieldViolations: connectError.findDetails(BadRequestSchema).flatMap(x => x.fieldViolations),
        requestInfo: connectError.findDetails(RequestInfoSchema),
    };
    if (defaultMessagePreffix) res.message = `${defaultMessagePreffix}: ${res.message}`;

    const isAuthError: boolean = [Code.PermissionDenied, Code.Unauthenticated].includes(connectError.code);
    if (isAuthError) window.setTimeout(() => router.navigate(URLS.auth.login));

    return res;
}

// Given how the form errors are constructed and transmitted through gRPC,
// we can have multiple errors per field, so we need to account for that
// in our unified form errors type.
export function parseFormErrors<Input extends Rec>(
    exception: BareException,
    // String allows usage of `Object.keys(Input)` and keyof provides intelisense
    knownFields?: string[] | Array<keyof Input>,
): FormErrors<Input> {
    const { message, fieldViolations } = parseError(exception);

    const global: string[] = [];
    if (message) global.push(message);

    const { unmatched, parsed } = parseFieldViolations(fieldViolations, knownFields);
    if (unmatched.length) global.push(...unmatched);
    const fields: FieldBasedErrors<Input> = parsed;

    return { global, fields };
}

export function renderFieldErrorsAsList(fieldErrors: Maybe<FieldErrors>): null | string {
    if (!fieldErrors) return null;
    if (fieldErrors.length === 1) return fieldErrors[0];
    return fieldErrors.map(x => `- ${x}`).join('\n');
}

export function collectAllErrors(error: unknown | Error | ConnectError): null | string[] {
    const $ = parseFormErrors(error, []);
    return $.global ?? null;
}

export function collectAllErrorsAsFormattedList(error: unknown | Error | ConnectError): null | string {
    const $ = collectAllErrors(error);
    return renderFieldErrorsAsList($);
}

export { hasFormErrors };
