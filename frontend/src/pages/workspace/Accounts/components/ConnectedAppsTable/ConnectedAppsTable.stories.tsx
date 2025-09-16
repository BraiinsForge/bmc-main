import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { ConnectedAccountsTable, type ConnectedAccountsTableProps } from './index';

export default {
    title: 'accounts/Table',
    component: ConnectedAccountsTable,
    args: {
        accounts: [pb.create(pb.AccountSchema, {}), pb.create(pb.AccountSchema, {}), pb.create(pb.AccountSchema, {})],
        onDelete: action('onDelete'),
        onEdit: action('onEdit'),
    } satisfies ConnectedAccountsTableProps,
} satisfies Meta<ConnectedAccountsTableProps>;

export function Table(args: ConnectedAccountsTableProps) {
    return <ConnectedAccountsTable {...args} />;
}
