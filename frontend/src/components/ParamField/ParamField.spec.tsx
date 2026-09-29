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

import { beforeEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, render, fireEvent } from '@testing-library/react/pure';
import { useState } from 'react';
import { IntlProvider } from 'react-intl';
import * as pb from '@/proto';
import { ParamField } from './ParamField';
import { listItem, type FieldValue, type ListItem, type RowError } from './value';

beforeEach(cleanup);

const stringField = (format: pb.StringFormat) =>
    pb.create(pb.ManifestParamDefinitionSchema, {
        key: 'token',
        name: 'API Token',
        kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, { format }) },
    });

const renderField = (format: pb.StringFormat, value = 'sk-secret') =>
    render(
        <IntlProvider locale="en">
            <ParamField id="f" definition={stringField(format)} value={value} onChange={() => {}} timezones={[]} />
        </IntlProvider>,
    );

const input = () => document.body.querySelector<HTMLInputElement>('#f');

describe('ParamField password format', () => {
    test('hides the value until revealed', () => {
        renderField(pb.StringFormat.PASSWORD);
        expect(input()?.type).toBe('password');
    });

    test('reveals the value on toggle, and hides it again', () => {
        const { getByRole } = renderField(pb.StringFormat.PASSWORD);
        const toggle = getByRole('button');

        fireEvent.click(toggle);
        expect(input()?.type).toBe('text');

        fireEvent.click(toggle);
        expect(input()?.type).toBe('password');
    });

    test('a plain string format has no reveal toggle', () => {
        const { queryByRole } = renderField(pb.StringFormat.UNSPECIFIED);

        expect(input()?.type).toBe('text');
        expect(queryByRole('button')).toBeNull();
    });
});

const listField = pb.create(pb.ManifestParamDefinitionSchema, {
    key: 'symbols',
    name: 'Symbols',
    kind: {
        case: 'paramArray',
        value: pb.create(pb.ParamArraySchema, {
            items: { kind: { case: 'paramString', value: { defaultValue: 'BTC' } } },
            minItems: 1,
            maxItems: 2,
        }),
    },
});

const linksField = pb.create(pb.ManifestParamDefinitionSchema, {
    key: 'links',
    name: 'Links',
    kind: {
        case: 'paramArray',
        value: pb.create(pb.ParamArraySchema, {
            items: {
                kind: {
                    case: 'paramObject',
                    value: {
                        fields: [
                            { key: 'label', name: 'Label', kind: { case: 'paramString', value: {} } },
                            {
                                key: 'url',
                                name: 'URL',
                                isOptional: true,
                                kind: { case: 'paramString', value: { defaultValue: 'https://' } },
                            },
                        ],
                    },
                },
            },
            maxItems: 3,
        }),
    },
});

function renderList(value: ListItem[], itemErrors?: Array<RowError | undefined>, definition = listField) {
    const onChange = rstest.fn<(key: string, value: FieldValue) => void>();
    const view = render(
        <IntlProvider locale="en">
            <ParamField
                id="f"
                definition={definition}
                value={value}
                itemErrors={itemErrors}
                onChange={onChange}
                timezones={[]}
            />
        </IntlProvider>,
    );
    return { ...view, onChange };
}

/** Feeds every change back in, as the params form does, so an added row actually renders. */
function LiveList(props: { initial: ListItem[]; definition: pb.ManifestParamDefinition }) {
    const [value, setValue] = useState(props.initial);
    return (
        <IntlProvider locale="en">
            <ParamField
                id="f"
                definition={props.definition}
                value={value}
                onChange={(_, next) => setValue(next as ListItem[])}
                timezones={[]}
            />
        </IntlProvider>
    );
}

describe('ParamField list', () => {
    test('renders an input per row', () => {
        const { getByLabelText } = renderList([listItem('NVDA'), listItem('AAPL')]);
        expect(getByLabelText('Symbols, item 1')).toHaveProperty('value', 'NVDA');
        expect(getByLabelText('Symbols, item 2')).toHaveProperty('value', 'AAPL');
    });

    test('adds a row seeded from the item default', () => {
        const row = listItem('NVDA');
        const { getByRole, onChange } = renderList([row]);
        fireEvent.click(getByRole('button', { name: 'Add' }));
        const [key, value] = onChange.mock.calls[0];
        expect(key).toBe('symbols');
        expect((value as ListItem[]).map(x => x.value)).toEqual(['NVDA', 'BTC']);
    });

    test('focuses the input of an added row', () => {
        const { getByRole, getByLabelText } = render(<LiveList initial={[listItem('NVDA')]} definition={listField} />);
        fireEvent.click(getByRole('button', { name: 'Add' }));
        expect(document.activeElement).toBe(getByLabelText('Symbols, item 2'));
    });

    test('removes the row whose minus was clicked', () => {
        const [first, second] = [listItem('NVDA'), listItem('AAPL')];
        const { getAllByRole, onChange } = renderList([first, second]);
        fireEvent.click(getAllByRole('button', { name: 'Remove' })[0]);
        expect(onChange).toHaveBeenCalledWith('symbols', [second]);
    });

    test('offers no add at max_items and no remove at min_items', () => {
        const full = renderList([listItem('NVDA'), listItem('AAPL')]);
        expect(full.getByRole('button', { name: 'Add' })).toHaveProperty('disabled', true);
        cleanup();

        const least = renderList([listItem('NVDA')]);
        expect(least.getByRole('button', { name: 'Remove' })).toHaveProperty('disabled', true);
    });

    test('a toggle row is named by its hidden label and keeps its On/Off text', () => {
        const flags = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'flags',
            name: 'Flags',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, {
                    items: { kind: { case: 'paramBoolean', value: {} } },
                    maxItems: 2,
                }),
            },
        });
        const { getByRole, getByText } = render(
            <IntlProvider locale="en">
                <ParamField id="f" definition={flags} value={[listItem(true)]} onChange={() => {}} timezones={[]} />
            </IntlProvider>,
        );
        expect(getByRole('switch', { name: 'Flags, item 1' })).toBeTruthy();
        expect(getByText('On')).toBeTruthy();
    });

    test("shows an item's error on its own row", () => {
        const { getByText } = renderList([listItem('NVDA'), listItem('')], [undefined, { error: 'Value is required' }]);
        expect(getByText('Value is required')).toBeTruthy();
    });
});

describe('ParamField object list', () => {
    test('renders an input per field of each row, under one header', () => {
        const { getByLabelText, getByText } = renderList(
            [listItem({ label: 'Pool', url: 'https://pool' })],
            undefined,
            linksField,
        );
        expect(getByLabelText('Links, item 1, Label')).toHaveProperty('value', 'Pool');
        expect(getByLabelText('Links, item 1, URL')).toHaveProperty('value', 'https://pool');
        expect(getByText('URL (optional)')).toBeTruthy();
    });

    test('adds a row seeded from each field default', () => {
        const { getByRole, onChange } = renderList([], undefined, linksField);
        fireEvent.click(getByRole('button', { name: 'Add' }));
        const [, value] = onChange.mock.calls[0];
        expect((value as ListItem[]).map(row => row.value)).toEqual([{ label: '', url: 'https://' }]);
    });

    test('focuses the first field of an added row, not its drag handle', () => {
        const { getByRole, getByLabelText } = render(<LiveList initial={[]} definition={linksField} />);
        fireEvent.click(getByRole('button', { name: 'Add' }));
        expect(document.activeElement).toBe(getByLabelText('Links, item 1, Label'));
    });

    test("shows a field's error on its own input", () => {
        const { getByText } = renderList(
            [listItem({ label: '', url: null })],
            [{ fields: { label: 'Value is required' } }],
            linksField,
        );
        expect(getByText('Value is required')).toBeTruthy();
    });
});
