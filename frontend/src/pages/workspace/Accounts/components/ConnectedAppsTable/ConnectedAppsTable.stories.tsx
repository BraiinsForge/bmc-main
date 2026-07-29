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

import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { ConnectedAccountsTable, type ConnectedAccountsTableProps } from './index';

const credentialTypes = new Map(
    [
        pb.create(pb.CredentialTypeSchema, { id: 'braiins-pool', name: 'Braiins Pool' }),
        pb.create(pb.CredentialTypeSchema, { id: 'generic-token', name: 'Token' }),
        pb.create(pb.CredentialTypeSchema, { id: 'generic-userpass', name: 'Username & password' }),
    ].map(t => [t.id, t]),
);

const accounts = [
    pb.create(pb.AccountSchema, { id: '1', typeId: 'braiins-pool', name: 'Primary pool' }),
    pb.create(pb.AccountSchema, { id: '2', typeId: 'generic-token', name: 'Weather API' }),
    pb.create(pb.AccountSchema, { id: '3', typeId: 'generic-userpass', name: 'Media server' }),
];

export default {
    title: 'Accounts/Table',
    component: ConnectedAccountsTable,
    args: {
        accounts,
        credentialTypes,
        onDelete: action('onDelete'),
        onEdit: action('onEdit'),
    } satisfies ConnectedAccountsTableProps,
} satisfies Meta<ConnectedAccountsTableProps>;

export function Table(args: ConnectedAccountsTableProps) {
    return <ConnectedAccountsTable {...args} />;
}
