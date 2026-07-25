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
import { CredentialTypeForm } from './CredentialTypeForm';

// Mirror the firmware built-ins in bmc/src/credential.rs.
function stringField(key: string, name: string, description: string, secret: boolean) {
    return pb.create(pb.ManifestParamDefinitionSchema, {
        key,
        name,
        description,
        isOptional: false,
        kind: {
            case: 'paramString',
            value: pb.create(pb.ParamStringSchema, secret ? { format: pb.StringFormat.PASSWORD } : {}),
        },
    });
}

const genericToken = pb.create(pb.CredentialTypeSchema, {
    id: 'generic-token',
    name: 'Token',
    description: 'A single API token or bearer secret.\n\n**The widget may send them to any host.**',
    fields: [stringField('token', 'Token', 'The API token or bearer secret.', true)],
});

const genericUserpass = pb.create(pb.CredentialTypeSchema, {
    id: 'generic-userpass',
    name: 'Username & password',
    description: 'A username and password pair.\n\n**The widget may send them to any host.**',
    fields: [
        stringField('username', 'Username', 'The account username.', false),
        stringField('password', 'Password', 'The account password.', true),
    ],
});

const braiinsPool = pb.create(pb.CredentialTypeSchema, {
    id: 'braiins-pool',
    name: 'Braiins Pool',
    description: 'A Braiins Pool API token used to fetch your worker stats.',
    fields: [stringField('token', 'API token', 'Your Braiins Pool API token.', true)],
    egress: pb.create(pb.EgressPolicySchema, { allowHosts: ['api.braiins.com'] }),
});

export default {
    title: 'workspace/Accounts/CredentialTypeForm',
    component: CredentialTypeForm,
};

export const BraiinsPool = () => (
    <div className="ui-box">
        <CredentialTypeForm type={braiinsPool} />
    </div>
);
export const GenericToken = () => (
    <div className="ui-box">
        <CredentialTypeForm type={genericToken} />
    </div>
);
export const GenericUserpass = () => (
    <div className="ui-box">
        <CredentialTypeForm type={genericUserpass} />
    </div>
);
