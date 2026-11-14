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

import { hasFormErrors } from '@/lib/form';
import { toast } from '@/lib/toast';

import { RPC_CODES_MAP } from './types';
import type { BareException, RpcStatus, FieldBasedErrors, FormErrors, FieldErrors } from './types';

import { parseFieldViolations } from './parseFieldViolation';
import { BadRequestSchema, RequestInfoSchema } from '../gen/google/rpc/error_details_pb';

/**
 * `error` as the RPC status it carries, re-checking
 * the session when the device refused it for one.
 */
export function parseError(error: BareException, defaultMessagePreffix?: string): RpcStatus {
    const res = describeError(error, defaultMessagePreffix);
    const isAuthError: boolean = [Code.PermissionDenied, Code.Unauthenticated].includes(res.rpc_reason_code);
    if (isAuthError) {
        // Held from here, before the caller raises its toast,
        // and dropped if the session turns out to be gone.
        toast.holdErrors();
        void import('@/store')
            .then(({ store }) => store.recheckSession())
            .catch(($: unknown) => {
                // With no check to answer for them, the held toasts show.
                toast.releaseErrors(true);
                throw $;
            });
    }
    return res;
}

/** `error` as the RPC status it carries, leaving the session alone. */
export function describeError(error: BareException, defaultMessagePreffix?: string): RpcStatus {
    const connectError = ConnectError.from(error);

    const res: RpcStatus = {
        rpc_reason_name: RPC_CODES_MAP[connectError.code] || 'Unknown',
        rpc_reason_code: connectError.code || Code.Unknown,
        // Not `message`, which for an empty sentence is the bare code: `[invalid_argument]`
        message: connectError.rawMessage || null,

        // https://connectrpc.com/docs/web/errors/#error-details
        fieldViolations: connectError.findDetails(BadRequestSchema).flatMap(x => x.fieldViolations),
        requestInfo: connectError.findDetails(RequestInfoSchema),
    };
    if (defaultMessagePreffix) res.message = `${defaultMessagePreffix}: ${res.message}`;

    return res;
}

/**
 * The error's text for the user, none for a lost connection.
 * Logs an `Unknown` that no failed fetch explains.
 */
export function errorMessage(error: BareException): string | undefined {
    const { message, rpc_reason_code } = describeError(error);
    const lost = isConnectionLost(error);
    // No status from the device and no failed fetch:
    // a bug in the page or a garbled reply, worth a trace
    if (rpc_reason_code === Code.Unknown && !lost) console.error(error);
    return (!lost && message) || undefined;
}

export interface ParseFormErrorsOptions<Input extends Rec> {
    // The fields the form draws. Any other violation goes to `global`.
    // Without them, every violation lands on its path.
    // Plain strings take `Object.keys` of an empty request, `keyof` completes the names
    knownFields?: string[] | Array<keyof Input>;
    // Shown when the error says nothing a field or the page could show
    fallback: string;
}

// Given how the form errors are constructed and transmitted through gRPC,
// we can have multiple errors per field, so we need to account for that
// in our unified form errors type.
export function parseFormErrors<Input extends Rec>(
    exception: BareException,
    { knownFields, fallback }: ParseFormErrorsOptions<Input>,
): FormErrors<Input> {
    const { fieldViolations } = parseError(exception);
    const message = errorMessage(exception);

    const global: string[] = [];
    if (message) global.push(message);

    const { unmatched, parsed } = parseFieldViolations(fieldViolations, knownFields);
    if (unmatched.length) global.push(...unmatched);
    const fields: FieldBasedErrors<Input> = parsed;

    const errors = { global, fields };
    if (!hasFormErrors(errors)) global.push(fallback);
    return errors;
}

function formatList(errors: FieldErrors): string {
    if (errors.length === 1) return errors[0];
    return errors.map(x => `- ${x}`).join('\n');
}

export function renderFieldErrorsAsList(fieldErrors: Maybe<FieldErrors>): null | string {
    return fieldErrors ? formatList(fieldErrors) : null;
}

/**
 * The browser's failed `fetch`, which connect wraps as `Unknown` around the fetch's `TypeError`.
 * A bug in the page's own code arrives bare, and the device words its `Unavailable` itself.
 */
export function isConnectionLost(error: unknown): boolean {
    return error instanceof ConnectError && error.code === Code.Unknown && error.cause instanceof TypeError;
}

/** Every reason `error` gives, or the `fallback` when it gives none. */
export function collectAllErrors(error: unknown, fallback: string): string[] {
    return parseFormErrors(error, { knownFields: [], fallback }).global;
}

/** `collectAllErrors` as one string, a list when there are several. */
export function collectAllErrorsAsFormattedList(error: unknown, fallback: string): string {
    return formatList(collectAllErrors(error, fallback));
}

export { hasFormErrors };
