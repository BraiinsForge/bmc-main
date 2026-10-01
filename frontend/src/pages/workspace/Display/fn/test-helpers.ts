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

import { Code, ConnectError } from '@connectrpc/connect';

import * as pb from '@/proto';
import { BadRequestSchema } from '@/proto/gen/google/rpc/error_details_pb';
import { assertUnreachable } from '@/lib/ts';

/** A server refusal carrying one field violation, as the scene handlers send it. */
export function badRequest(field: string, description: string): ConnectError {
    const detail = pb.create(BadRequestSchema, { fieldViolations: [{ field, description }] });
    return new ConnectError('Bad request', Code.InvalidArgument, undefined, [
        { desc: BadRequestSchema, value: detail },
    ]);
}

export function paramDef(
    kindCase: pb.ManifestParamDefinition['kind']['case'],
    key = 'k',
    isOptional = false,
    overrides: Record<string, any> = {},
): pb.ManifestParamDefinition {
    let kind: pb.ManifestParamDefinition['kind'];
    switch (kindCase) {
        case 'paramString':
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema, overrides) };
            break;
        case 'paramInteger':
            kind = { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, overrides) };
            break;
        case 'paramDouble':
            kind = { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema, overrides) };
            break;
        case 'paramBoolean':
            kind = { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, overrides) };
            break;
        case 'paramTimezone':
            kind = { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema, overrides) };
            break;
        case 'paramArray':
            kind = { case: 'paramArray', value: pb.create(pb.ParamArraySchema, overrides) };
            break;
        case undefined:
            kind = { case: undefined };
            break;
        default:
            assertUnreachable(kindCase, 'paramDef kind');
    }
    return pb.create(pb.ManifestParamDefinitionSchema, { key, name: 'K', isOptional, kind });
}
