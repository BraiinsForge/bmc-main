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

import { beforeEach, describe, expect, test } from '@rstest/core';
import { cleanup, render as renderBare, fireEvent } from '@testing-library/react/pure';
import { IntlContext } from 'react-intl';

import * as pb from '@/proto';
import { URLS } from '@/constants';
import { fakeIntlProp } from '@/mocks/intl';
import { withRouter } from '@/mocks/router';

import type { FormifiedValue, ParamsFormErrors } from '../../fn';
import { paramDef } from '../../fn/test-helpers';
import { FormWidgetManifest } from './FormWidgetManifest';

beforeEach(cleanup);

const wrapped = (ui: ReactElement) => <IntlContext.Provider value={fakeIntlProp} children={withRouter(ui)} />;

/// `rerender` is wrapped too: handed a bare tree it would drop the providers
/// and remount, where a test watching a prop change needs the same tree updated.
const render = (ui: ReactElement) => {
    const result = renderBare(wrapped(ui));
    return { ...result, rerender: (next: ReactElement) => result.rerender(wrapped(next)) };
};

describe('FormWidgetManifest', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'w',
        name: 'W',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [paramDef('paramInteger', 'count')],
    });

    test('renders per-field error from errors.fields[key]', () => {
        const { getByText } = render(
            <FormWidgetManifest
                isOpen
                onSave={() => {}}
                onCancel={() => {}}
                manifest={manifest}
                params={{ count: 'abc' }}
                errors={{ global: [], fields: { count: ['Not an integer'] } }}
                onParamChange={() => {}}
                timezones={[]}
            />,
        );
        expect(getByText('Not an integer')).toBeTruthy();
    });

    test('onParamChange propagates numeric input as a string', () => {
        let captured: [string, FormifiedValue] | null = null;
        render(
            <FormWidgetManifest
                isOpen
                onSave={() => {}}
                onCancel={() => {}}
                manifest={manifest}
                params={{ count: '' }}
                errors={null}
                onParamChange={(k, v) => {
                    captured = [k, v];
                }}
                timezones={[]}
            />,
        );
        const input = document.body.querySelector<HTMLInputElement>('input[type="number"]');
        expect(input).toBeTruthy();
        if (!input) throw new Error('numeric input not found');
        fireEvent.change(input, { target: { value: '42' } });
        expect(captured).toEqual(['count', '42']);
    });
});

describe('FormWidgetManifest credential slots', () => {
    const slotManifest = (required: boolean) =>
        pb.create(pb.WidgetManifestSchema, {
            uid: 'w',
            name: 'W',
            supportedSizes: [pb.WidgetSize.FULL],
            credentials: [
                pb.create(pb.CredentialSlotDefinitionSchema, {
                    key: 'pool',
                    typeId: 'braiins-pool',
                    label: 'Pool Account',
                    required,
                }),
            ],
        });

    const account = (id: string, name: string, typeId: string) =>
        pb.create(pb.AccountSchema, {
            id,
            name,
            typeId,
            createdAt: pb.create(pb.TimestampSchema, { seconds: 1_700_000_000n }),
        });

    const ACCOUNTS = [
        account('a1', 'Pool One', 'braiins-pool'),
        account('a2', 'Pool Two', 'braiins-pool'),
        account('t1', 'Some Token', 'generic-token'),
    ];

    // Each option row is icon + name span + created-at; the name is its first span.
    const optionNames = () =>
        Array.from(document.body.querySelectorAll('[role="option"]')).map(o => o.querySelector('span')?.textContent);

    const optionByName = (name: string) =>
        Array.from(document.body.querySelectorAll('[role="option"]')).find(
            o => o.querySelector('span')?.textContent === name,
        );

    const renderSlots = (props: {
        required?: boolean;
        bindings?: Record<string, string>;
        errors?: ParamsFormErrors;
        accounts?: pb.Account[];
        onChange?(slotKey: string, accountId: string): void;
    }) =>
        render(
            <FormWidgetManifest
                isOpen
                onSave={() => {}}
                onCancel={() => {}}
                manifest={slotManifest(props.required ?? false)}
                params={{}}
                errors={props.errors ?? null}
                onParamChange={() => {}}
                timezones={[]}
                accounts={props.accounts ?? ACCOUNTS}
                credentialBindings={props.bindings ?? {}}
                onCredentialBindingChange={props.onChange ?? (() => {})}
            />,
        );

    test('offers only accounts whose type matches the slot, plus an unbind entry', () => {
        const { getByRole } = renderSlots({});
        fireEvent.click(getByRole('combobox'));

        expect(optionNames()).toEqual(['— None —', 'Pool One', 'Pool Two']);
    });

    test('selecting an account propagates its id for the slot', () => {
        let captured: [string, string] | null = null;
        const { getByRole } = renderSlots({
            onChange: (slotKey, accountId) => {
                captured = [slotKey, accountId];
            },
        });
        fireEvent.click(getByRole('combobox'));
        const option = optionByName('Pool Two');
        if (!option) throw new Error('option not found');
        fireEvent.click(option);

        expect(captured).toEqual(['pool', 'a2']);
    });

    test('selecting the unbind entry propagates an empty id', () => {
        let captured: [string, string] | null = null;
        const { getByRole } = renderSlots({
            bindings: { pool: 'a1' },
            onChange: (slotKey, accountId) => {
                captured = [slotKey, accountId];
            },
        });
        fireEvent.click(getByRole('combobox'));
        const option = optionByName('— None —');
        if (!option) throw new Error('unbind option not found');
        fireEvent.click(option);

        expect(captured).toEqual(['pool', '']);
    });

    test('the shown selection follows a changed binding', () => {
        const props = (bindings: Record<string, string>) => (
            <FormWidgetManifest
                isOpen
                onSave={() => {}}
                onCancel={() => {}}
                manifest={slotManifest(false)}
                params={{}}
                errors={null}
                onParamChange={() => {}}
                timezones={[]}
                accounts={ACCOUNTS}
                credentialBindings={bindings}
                onCredentialBindingChange={() => {}}
            />
        );

        const { getByRole, rerender } = render(props({ pool: 'a1' }));
        expect(getByRole('combobox').textContent).toContain('Pool One');

        rerender(props({ pool: 'a2' }));
        expect(getByRole('combobox').textContent).toContain('Pool Two');

        rerender(props({}));
        expect(getByRole('combobox').textContent).toContain('— None —');
    });

    test('an account of the wrong type never satisfies a slot binding', () => {
        // Reachable from a hand-edited config: `effective_bindings` drops a binding
        // whose account is gone, but keeps one that is merely mistyped.
        const { queryByText, getByRole } = renderSlots({ bindings: { pool: 't1' } });

        expect(queryByText('Takes a braiins-pool account — pick another, or clear it.')).toBeTruthy();
        expect(getByRole('combobox').textContent).not.toContain('Some Token');
    });

    test('a mistyped binding blocks saving', () => {
        // Unlike an unbound slot, which saves deliberately, this one the server refuses —
        // so offering the click only earns a toast after the fact.
        const { getByText } = renderSlots({ bindings: { pool: 't1' } });
        const done = getByText('Done').closest('button');
        expect(done?.disabled).toBe(true);
    });

    test('the stand-in for a mistyped binding is not dressed as an account', () => {
        // It carries the bound id so it can be selected on its own,
        // which is exactly what made it render with an icon and a blank created-at.
        const { getByRole } = renderSlots({ bindings: { pool: 't1' } });
        fireEvent.click(getByRole('combobox'));

        const option = optionByName('— Invalid —');
        if (!option) throw new Error('stand-in option not found');
        expect(option.querySelector('[class*="accountElement"]')).toBeNull();
    });

    test('a mistyped binding can be cleared', () => {
        // Without an item of its own it would share `— None —`,
        // so picking that would re-select what is already showing and never fire,
        // leaving an id that only fails on save.
        let captured: [string, string] | null = null;
        const { getByRole } = renderSlots({
            bindings: { pool: 't1' },
            onChange: (slotKey, accountId) => {
                captured = [slotKey, accountId];
            },
        });
        fireEvent.click(getByRole('combobox'));

        expect(optionNames()).toContain('— Invalid —');

        const option = optionByName('— None —');
        if (!option) throw new Error('unbind option not found');
        fireEvent.click(option);

        expect(captured).toEqual(['pool', '']);
    });

    test('a server violation for the slot shows on its picker', () => {
        const { queryByText } = renderSlots({
            bindings: { pool: 'a1' },
            errors: { global: [], fields: {}, credentials: { pool: ['Account not found'] } },
        });

        expect(queryByText('Account not found')).toBeTruthy();
    });

    test('warns when a required slot is unbound', () => {
        const { queryByText } = renderSlots({ required: true });

        expect(queryByText('No account bound')).toBeTruthy();
        // Substituted, so the label lands in the sentence exactly once:
        // the wording must not repeat a noun the slot label already carries.
        expect(queryByText('Bind a Pool Account for this widget to work.')).toBeTruthy();
    });

    test('drops the warning once a required slot is bound', () => {
        const { queryByText } = renderSlots({ required: true, bindings: { pool: 'a1' } });
        expect(queryByText('No account bound')).toBeNull();
    });

    test('an unbound required slot still leaves saving enabled', () => {
        const { getByText } = renderSlots({ required: true });
        const done = getByText('Done').closest('button');
        expect(done?.disabled).toBe(false);
    });

    test('points at the accounts page when no account fits the slot', () => {
        const { queryByText, getByText } = renderSlots({ accounts: [account('t1', 'Some Token', 'generic-token')] });

        expect(queryByText('No matching account')).toBeTruthy();
        expect(getByText('add one in Accounts').closest('a')?.getAttribute('href')).toBe(URLS.pages.accounts);
    });

    test('the empty state outranks the unbound-required warning', () => {
        // Both apply at once on a fresh device; "bind one" is useless advice
        // while there is nothing to bind.
        const { queryByText } = renderSlots({ required: true, accounts: [] });

        expect(queryByText('No matching account')).toBeTruthy();
        expect(queryByText('No account bound')).toBeNull();
    });

    test('two slots of one type say it once, and the second stays quiet', () => {
        const twoPools = pb.create(pb.WidgetManifestSchema, {
            uid: 'w',
            name: 'W',
            supportedSizes: [pb.WidgetSize.FULL],
            credentials: [
                pb.create(pb.CredentialSlotDefinitionSchema, {
                    key: 'pool',
                    typeId: 'braiins-pool',
                    label: 'Pool Account',
                    required: true,
                }),
                pb.create(pb.CredentialSlotDefinitionSchema, {
                    key: 'pool_backup',
                    typeId: 'braiins-pool',
                    label: 'Backup Pool Account',
                    required: true,
                }),
            ],
        });
        const { queryAllByText } = render(
            <FormWidgetManifest
                isOpen
                onSave={() => {}}
                onCancel={() => {}}
                manifest={twoPools}
                params={{}}
                errors={null}
                onParamChange={() => {}}
                timezones={[]}
                accounts={[]}
                credentialBindings={{}}
                onCredentialBindingChange={() => {}}
            />,
        );

        expect(queryAllByText('No matching account')).toHaveLength(1);
        // Nor may the second fall back to "bind one" — there is still nothing to bind.
        expect(queryAllByText('No account bound')).toHaveLength(0);
    });
});
