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

import { useState } from 'react';
import * as pb from '@/proto';
import type { FieldValue } from '@/components';
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
    description: 'A single API token or bearer secret.',
    fields: [stringField('token', 'Token', 'The API token or bearer secret.', true)],
});

const genericUserpass = pb.create(pb.CredentialTypeSchema, {
    id: 'generic-userpass',
    name: 'Username & password',
    description: 'A username and password pair.',
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
    title: 'Accounts/CredentialTypeForm',
    component: CredentialTypeForm,
};

function Demo({ type }: { type: pb.CredentialType }) {
    const [values, setValues] = useState<Record<string, FieldValue>>({});
    return (
        <div className="ui-box" style={{ minWidth: 600 }}>
            <CredentialTypeForm
                type={type}
                values={values}
                onChange={(key, value) => setValues(prev => ({ ...prev, [key]: value }))}
            />
        </div>
    );
}

export const BraiinsPool = () => <Demo type={braiinsPool} />;
export const GenericToken = () => <Demo type={genericToken} />;
export const GenericUserpass = () => <Demo type={genericUserpass} />;
