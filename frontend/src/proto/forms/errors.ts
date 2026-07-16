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

import { ConnectError, Code } from '@connectrpc/connect';
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
    if (isAuthError) import('@/routes').then(({ default: router }) => router.navigate(URLS.auth.login));

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
