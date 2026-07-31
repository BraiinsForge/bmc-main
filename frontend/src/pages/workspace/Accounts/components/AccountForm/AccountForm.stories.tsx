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
import { AccountForm } from './AccountForm';

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

const credentialTypeList = [
    pb.create(pb.CredentialTypeSchema, {
        id: 'braiins-pool',
        name: 'Braiins Pool',
        description: 'A Braiins Pool API token used to fetch your worker stats.',
        fields: [stringField('token', 'API token', 'Your Braiins Pool API token.', true)],
        egress: pb.create(pb.EgressPolicySchema, { allowHosts: ['api.braiins.com'] }),
    }),
    pb.create(pb.CredentialTypeSchema, {
        id: 'generic-token',
        name: 'Token',
        description: 'A single API token or bearer secret.',
        fields: [stringField('token', 'Token', 'The API token or bearer secret.', true)],
    }),
    pb.create(pb.CredentialTypeSchema, {
        id: 'generic-userpass',
        name: 'Username & password',
        description: 'A username and password pair.',
        fields: [
            stringField('username', 'Username', 'The account username.', false),
            stringField('password', 'Password', 'The account password.', true),
        ],
    }),
];

const credentialTypes = new Map(credentialTypeList.map(t => [t.id, t]));

export default {
    title: 'Accounts/AccountForm',
    component: AccountForm,
};

function Demo({
    mode,
    typeId,
    error,
    fieldErrors,
    allowHostsError,
}: {
    mode: 'create' | 'edit';
    typeId: string;
    error?: string;
    fieldErrors?: Record<string, string[]>;
    allowHostsError?: string;
}) {
    const [type, setType] = useState(typeId);
    const [name, setName] = useState(mode === 'edit' ? 'My account' : '');
    const [values, setValues] = useState<Record<string, FieldValue>>({});
    const [allowHosts, setAllowHosts] = useState('');

    return (
        <div className="ui-box" style={{ minWidth: 600, maxWidth: '32rem' }}>
            <AccountForm
                mode={mode}
                credentialTypes={credentialTypes}
                type={{ value: type, onChange: setType }}
                name={{ value: name, onChange: setName }}
                fieldValues={values}
                onFieldChange={(key, value) => setValues(prev => ({ ...prev, [key]: value }))}
                fieldErrors={fieldErrors}
                allowHosts={{ value: allowHosts, onChange: setAllowHosts, error: allowHostsError }}
                error={error}
            />
        </div>
    );
}

export const CreateBraiinsPool = () => <Demo mode="create" typeId="braiins-pool" />;
export const CreateUsernamePassword = () => <Demo mode="create" typeId="generic-userpass" />;
export const Edit = () => <Demo mode="edit" typeId="braiins-pool" />;
export const WithError = () => (
    <Demo mode="create" typeId="generic-token" fieldErrors={{ token: ['Invalid API key.'] }} />
);
// The two halves of the destination control: a pinned type states where its secret
// goes, an unpinned one lets the operator say — and can reject what they wrote.
export const PinnedDestination = () => <Demo mode="create" typeId="braiins-pool" />;
export const RejectedDestination = () => (
    <Demo mode="create" typeId="generic-token" allowHostsError="Line 2: an entry cannot contain spaces" />
);
