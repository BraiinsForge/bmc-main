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
import { cleanup, render, fireEvent } from '@testing-library/react/pure';
import { IntlContext } from 'react-intl';
import * as pb from '@/proto';
import type { FormifiedValue, ParamsFormErrors } from '../../fn';
import { paramDef } from '../../fn/test-helpers';
import { FormWidgetManifest } from './FormWidgetManifest';

if (typeof ResizeObserver === 'undefined') {
    global.ResizeObserver = class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
    };
}

beforeEach(cleanup);

const fakeIntl = {
    formatMessage: ({ defaultMessage }: { defaultMessage?: string; id?: string }) => defaultMessage ?? '',
    locale: 'en',
    defaultLocale: 'en',
    timeZone: undefined,
    formats: {},
    defaultFormats: {},
    messages: {},
    onError: () => {},
} as unknown as ReturnType<typeof import('react-intl').useIntl>;

const wrap = (ui: ReactElement) => <IntlContext.Provider value={fakeIntl}>{ui}</IntlContext.Provider>;

describe('FormWidgetManifest', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'w',
        name: 'W',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [paramDef('paramInteger', 'count')],
    });

    test('renders per-field error from errors.fields[key]', () => {
        const { getByText } = render(
            wrap(
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
            ),
        );
        expect(getByText('Not an integer')).toBeTruthy();
    });

    test('onParamChange propagates numeric input as a string', () => {
        let captured: [string, FormifiedValue] | null = null;
        render(
            wrap(
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
            ),
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
        onChange?(slotKey: string, accountId: string): void;
    }) =>
        render(
            wrap(
                <FormWidgetManifest
                    isOpen
                    onSave={() => {}}
                    onCancel={() => {}}
                    manifest={slotManifest(props.required ?? false)}
                    params={{}}
                    errors={props.errors ?? null}
                    onParamChange={() => {}}
                    timezones={[]}
                    accounts={ACCOUNTS}
                    credentialBindings={props.bindings ?? {}}
                    onCredentialBindingChange={props.onChange ?? (() => {})}
                />,
            ),
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

        const { getByRole, rerender } = render(wrap(props({ pool: 'a1' })));
        expect(getByRole('combobox').textContent).toContain('Pool One');

        rerender(wrap(props({ pool: 'a2' })));
        expect(getByRole('combobox').textContent).toContain('Pool Two');

        rerender(wrap(props({})));
        expect(getByRole('combobox').textContent).toContain('— None —');
    });

    test('a binding whose account is gone reads as broken, not as unbound', () => {
        const { queryByText, getByRole } = renderSlots({ bindings: { pool: 'deleted-account' } });

        expect(queryByText('Bound account is gone')).toBeTruthy();
        expect(getByRole('combobox').textContent).not.toContain('deleted-account');
    });

    test('an account of the wrong type never satisfies a slot binding', () => {
        const { queryByText } = renderSlots({ bindings: { pool: 't1' } });
        expect(queryByText('Bound account is gone')).toBeTruthy();
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
});
