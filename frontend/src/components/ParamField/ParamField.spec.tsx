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
import { BoundRadioGroup, ParamField } from './ParamField';
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

describe('ParamField date format', () => {
    test('the ISO date it stores passes the input’s own pattern', () => {
        renderField(pb.StringFormat.DATE, '2026-01-01');
        expect(input()?.validity.patternMismatch).toBe(false);
    });
});

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

const radioGroup = (value: string | null) => (
    <BoundRadioGroup<string>
        id="mode"
        labelText="Mode"
        items={[
            { value: 'a', label: 'Option A' },
            { value: 'b', label: 'Option B' },
        ]}
        value={value}
        onChange={() => {}}
    />
);

const checkedValues = () =>
    Array.from(document.body.querySelectorAll<HTMLInputElement>('input[type="radio"]'))
        .filter(radio => radio.checked)
        .map(radio => radio.value);

describe('BoundRadioGroup selection follows the value prop', () => {
    test('a set value checks its radio', () => {
        render(radioGroup('a'));
        expect(checkedValues()).toEqual(['a']);
    });

    test('changing the value moves the check', () => {
        const { rerender } = render(radioGroup('a'));
        rerender(radioGroup('b'));
        expect(checkedValues()).toEqual(['b']);
    });

    test('clearing the value unchecks everything', () => {
        const { rerender } = render(radioGroup('a'));
        rerender(radioGroup(null));
        expect(checkedValues()).toEqual([]);
    });

    /// A click seeds the group's internal selection; switching the editor
    /// to another widget must still win over that remembered click.
    test('an entity switch after an accepted click still applies', () => {
        const { rerender } = render(radioGroup('a'));
        const radioB = document.body.querySelector<HTMLInputElement>('input[type="radio"][value="b"]');
        if (!radioB) throw new Error('BUG: radio b must render');
        fireEvent.click(radioB);
        rerender(radioGroup('b'));

        rerender(radioGroup('a'));
        expect(checkedValues()).toEqual(['a']);
    });
});

const periods = (enumControl: pb.EnumControl) =>
    pb.create(pb.ParamStringSchema, {
        enumControl,
        enumValues: [
            pb.create(pb.StringOptionSchema, { value: '1d', label: '1 Day' }),
            pb.create(pb.StringOptionSchema, { value: '7d', label: '7 Days' }),
        ],
    });

const periodField = (enumControl: pb.EnumControl) =>
    pb.create(pb.ManifestParamDefinitionSchema, {
        key: 'period',
        name: 'Time Period',
        kind: { case: 'paramString', value: periods(enumControl) },
    });

const renderEnum = (definition: pb.ManifestParamDefinition, value: FieldValue) =>
    render(
        <IntlProvider locale="en">
            <ParamField id="f" definition={definition} value={value} onChange={() => {}} timezones={[]} />
        </IntlProvider>,
    );

describe('ParamField enum control', () => {
    test('radio draws each option as a radio', () => {
        const { getByRole } = renderEnum(periodField(pb.EnumControl.RADIO), '7d');
        expect(getByRole('radio', { name: '7 Days' })).toHaveProperty('checked', true);
        expect(getByRole('radio', { name: '1 Day' })).toHaveProperty('checked', false);
    });

    test('an unset control keeps the dropdown', () => {
        const { queryAllByRole } = renderEnum(periodField(pb.EnumControl.UNSPECIFIED), '7d');
        expect(queryAllByRole('radio')).toHaveLength(0);
    });

    test('a number enum draws radios too', () => {
        const columns = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'columns',
            name: 'Columns',
            kind: {
                case: 'paramInteger',
                value: pb.create(pb.ParamIntegerSchema, {
                    enumControl: pb.EnumControl.RADIO,
                    enumValues: [
                        pb.create(pb.IntegerOptionSchema, { value: 1, label: 'One' }),
                        pb.create(pb.IntegerOptionSchema, { value: 2, label: 'Two' }),
                    ],
                }),
            },
        });
        const { getByRole } = renderEnum(columns, '2');
        expect(getByRole('radio', { name: 'Two' })).toHaveProperty('checked', true);
    });

    test('a radio row in a list is named by its hidden label', () => {
        const periodList = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'periods',
            name: 'Periods',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, {
                    items: { kind: { case: 'paramString', value: periods(pb.EnumControl.RADIO) } },
                    maxItems: 2,
                }),
            },
        });
        const { getByRole } = renderEnum(periodList, [listItem('1d')]);
        expect(getByRole('group', { name: 'Periods, item 1' })).toBeTruthy();
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

/** The message an invalid input points at through `aria-errormessage`, as assistive tech reads it. */
function errorOf(input: HTMLElement): string | null {
    if (input.getAttribute('aria-invalid') !== 'true') return null;
    const id = input.getAttribute('aria-errormessage');
    return id ? (document.getElementById(id)?.textContent ?? null) : null;
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
        fireEvent.click(getByRole('button', { name: 'Add to Symbols' }));
        const [key, value] = onChange.mock.calls[0];
        expect(key).toBe('symbols');
        expect((value as ListItem[]).map(x => x.value)).toEqual(['NVDA', 'BTC']);
    });

    test('focuses the input of an added row', () => {
        const { getByRole, getByLabelText } = render(<LiveList initial={[listItem('NVDA')]} definition={listField} />);
        fireEvent.click(getByRole('button', { name: 'Add to Symbols' }));
        expect(document.activeElement).toBe(getByLabelText('Symbols, item 2'));
    });

    test('removes the row whose minus was clicked', () => {
        const [first, second] = [listItem('NVDA'), listItem('AAPL')];
        const { getByRole, onChange } = renderList([first, second]);
        fireEvent.click(getByRole('button', { name: 'Remove Symbols, item 1' }));
        expect(onChange).toHaveBeenCalledWith('symbols', [second]);
    });

    test('offers no add at max_items and no remove at min_items', () => {
        const full = renderList([listItem('NVDA'), listItem('AAPL')]);
        expect(full.getByRole('button', { name: 'Add to Symbols' })).toHaveProperty('disabled', true);
        cleanup();

        const least = renderList([listItem('NVDA')]);
        expect(least.getByRole('button', { name: 'Remove Symbols, item 1' })).toHaveProperty('disabled', true);
    });

    test('names its Add button after the list, keeping the visible text', () => {
        const { getByRole } = renderList([listItem('NVDA')]);
        expect(getByRole('button', { name: 'Add to Symbols' }).textContent).toBe('Add');
    });

    test("names each row's handle and remove button after the row", () => {
        const { getByRole } = renderList([listItem('NVDA'), listItem('AAPL')]);
        expect(getByRole('button', { name: 'Move Symbols, item 2' })).toBeTruthy();
        expect(getByRole('button', { name: 'Remove Symbols, item 2' })).toBeTruthy();
    });

    // A mouse click leaves no focus anywhere: `Button` blurs after every click.
    describe('focus after a removal from the keyboard', () => {
        const openList = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'symbols',
            name: 'Symbols',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, {
                    items: { kind: { case: 'paramString', value: {} } },
                    maxItems: 3,
                }),
            },
        });
        const live = (values: string[], definition = openList) =>
            render(<LiveList initial={values.map(listItem)} definition={definition} />);
        const press = (button: HTMLElement) => fireEvent.keyDown(button, { key: 'Enter' });

        test('moves to the remove button of the row that takes its place', () => {
            const { getByRole } = live(['A', 'B', 'C']);
            const next = getByRole('button', { name: 'Remove Symbols, item 2' });
            press(getByRole('button', { name: 'Remove Symbols, item 1' }));
            expect(document.activeElement).toBe(next);
        });

        test('moves to the row above when the last row goes', () => {
            const { getByRole } = live(['A', 'B']);
            const above = getByRole('button', { name: 'Remove Symbols, item 1' });
            press(getByRole('button', { name: 'Remove Symbols, item 2' }));
            expect(document.activeElement).toBe(above);
        });

        test('moves to the remaining input once nothing more can be removed', () => {
            const { getByRole, getByLabelText } = live(['A', 'B'], listField);
            press(getByRole('button', { name: 'Remove Symbols, item 1' }));
            expect(document.activeElement).toBe(getByLabelText('Symbols, item 1'));
        });

        test('moves to the next Remove button, not a field keyed remove', () => {
            const rows = pb.create(pb.ManifestParamDefinitionSchema, {
                key: 'rows',
                name: 'Rows',
                kind: {
                    case: 'paramArray',
                    value: pb.create(pb.ParamArraySchema, {
                        items: {
                            kind: {
                                case: 'paramObject',
                                value: {
                                    fields: [
                                        { key: 'remove', name: 'Remove', kind: { case: 'paramString', value: {} } },
                                    ],
                                },
                            },
                        },
                        maxItems: 3,
                    }),
                },
            });
            const initial = [listItem({ remove: 'a' }), listItem({ remove: 'b' })];
            const { getByRole } = render(<LiveList initial={initial} definition={rows} />);
            const next = getByRole('button', { name: 'Remove Rows, item 2' });
            press(getByRole('button', { name: 'Remove Rows, item 1' }));
            expect(document.activeElement).toBe(next);
        });

        test('moves to Add when the list empties', () => {
            const { getByRole } = live(['A']);
            press(getByRole('button', { name: 'Remove Symbols, item 1' }));
            expect(document.activeElement).toBe(getByRole('button', { name: 'Add to Symbols' }));
        });

        // jsdom 30 lets a disabled button with a tabindex take focus, checking tabindex
        // before disabled (`helpers/focusing.js`), where a browser refuses;
        // so the check is that Add was enabled when it took focus.
        test('moves to Add only once Add is enabled, at a one-item limit', () => {
            const oneItemList = pb.create(pb.ManifestParamDefinitionSchema, {
                key: 'symbols',
                name: 'Symbols',
                kind: {
                    case: 'paramArray',
                    value: pb.create(pb.ParamArraySchema, {
                        items: { kind: { case: 'paramString', value: {} } },
                        maxItems: 1,
                    }),
                },
            });
            const { getByRole } = live(['A'], oneItemList);
            const add = getByRole('button', { name: 'Add to Symbols' }) as HTMLButtonElement;
            const disabledWhenFocused: boolean[] = [];
            add.addEventListener('focus', () => disabledWhenFocused.push(add.disabled));
            press(getByRole('button', { name: 'Remove Symbols, item 1' }));
            expect(disabledWhenFocused).toEqual([false]);
        });
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

    const toggleItem: pb.ArrayItemKind['kind'] = { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema) };
    const shownRow: pb.ArrayItemKind['kind'] = {
        case: 'paramObject',
        value: pb.create(pb.ParamObjectSchema, {
            fields: [
                { key: 'label', name: 'Label', kind: { case: 'paramString', value: {} } },
                { key: 'shown', name: 'Shown', kind: { case: 'paramBoolean', value: {} } },
            ],
        }),
    };

    test.each<[string, pb.ArrayItemKind['kind'], ListItem[], RowError, string, string]>([
        [
            'a toggle row',
            toggleItem,
            [listItem(true), listItem(true)],
            { error: 'Repeats item 1' },
            'Flags, item 2',
            'Repeats item 1',
        ],
        [
            'a toggle field of an object row',
            shownRow,
            [listItem({ label: 'A', shown: true }), listItem({ label: 'B', shown: true })],
            { fields: { shown: 'Same as item 1' } },
            'Flags, item 2, Shown',
            'Same as item 1',
        ],
    ])("shows %s's error under the toggle", (_, kind, value, rowError, switchName, message) => {
        const flags = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'flags',
            name: 'Flags',
            kind: { case: 'paramArray', value: pb.create(pb.ParamArraySchema, { items: { kind }, maxItems: 2 }) },
        });
        const { getByRole } = renderList(value, [undefined, rowError], flags);
        expect(errorOf(getByRole('switch', { name: switchName }))).toBe(message);
    });

    test.each([
        ['text', { case: 'paramString', value: { placeholder: 'e.g. BTC or AAPL' } }, 'e.g. BTC or AAPL'],
        ['number', { case: 'paramInteger', value: { placeholder: 'e.g. 42' } }, 'e.g. 42'],
    ] as const)('shows the item placeholder in an empty %s row', (_, itemKind, placeholder) => {
        const hinted = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'symbols',
            name: 'Symbols',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, { items: { kind: itemKind }, maxItems: 2 }),
            },
        });
        const { getByLabelText } = renderList([listItem('')], undefined, hinted);
        expect(getByLabelText('Symbols, item 1')).toHaveProperty('placeholder', placeholder);
    });

    test("shows an item's error on its own row", () => {
        const { getByLabelText } = renderList(
            [listItem('NVDA'), listItem('')],
            [undefined, { error: 'Value is required' }],
        );
        expect(errorOf(getByLabelText('Symbols, item 2'))).toBe('Value is required');
        expect(errorOf(getByLabelText('Symbols, item 1'))).toBeNull();
    });
});

describe('ParamField units', () => {
    const seconds: pb.ArrayItemKind['kind'] = {
        case: 'paramInteger',
        value: pb.create(pb.ParamIntegerSchema, { unit: 's' }),
    };

    test.each([
        [false, 'Interval (s)'],
        [true, 'Interval (s, optional)'],
    ])('a number names its unit with the field (optional: %s)', (isOptional, label) => {
        const interval = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'interval',
            name: 'Interval',
            isOptional,
            kind: seconds,
        });
        const { getByLabelText } = render(
            <IntlProvider locale="en">
                <ParamField id="f" definition={interval} value="" onChange={() => {}} timezones={[]} />
            </IntlProvider>,
        );
        expect(getByLabelText(label)).toBeTruthy();
    });

    test("a list's rows carry its items' unit", () => {
        const durations = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'durations',
            name: 'Durations',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, { items: { kind: seconds }, maxItems: 2 }),
            },
        });
        const { getByLabelText } = renderList([listItem('5')], undefined, durations);
        expect(getByLabelText('Durations (s), item 1')).toBeTruthy();
    });

    test("an object column carries its field's unit", () => {
        const rows = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'rows',
            name: 'Rows',
            kind: {
                case: 'paramArray',
                value: pb.create(pb.ParamArraySchema, {
                    items: {
                        kind: {
                            case: 'paramObject',
                            value: { fields: [{ key: 'delay', name: 'Delay', kind: seconds }] },
                        },
                    },
                    maxItems: 2,
                }),
            },
        });
        const { getByLabelText, getByText } = renderList([listItem({ delay: '5' })], undefined, rows);
        expect(getByText('Delay (s)')).toBeTruthy();
        expect(getByLabelText('Rows, item 1, Delay (s)')).toBeTruthy();
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
        expect(getByLabelText('Links, item 1, URL (optional)')).toHaveProperty('value', 'https://pool');
        expect(getByText('URL (optional)')).toBeTruthy();
    });

    test('adds a row seeded from each field default', () => {
        const { getByRole, onChange } = renderList([], undefined, linksField);
        fireEvent.click(getByRole('button', { name: 'Add to Links' }));
        const [, value] = onChange.mock.calls[0];
        expect((value as ListItem[]).map(row => row.value)).toEqual([{ label: '', url: 'https://' }]);
    });

    test('focuses the first field of an added row, not its drag handle', () => {
        const { getByRole, getByLabelText } = render(<LiveList initial={[]} definition={linksField} />);
        fireEvent.click(getByRole('button', { name: 'Add to Links' }));
        expect(document.activeElement).toBe(getByLabelText('Links, item 1, Label'));
    });

    test("shows a field's error on its own input", () => {
        const { getByLabelText } = renderList(
            [listItem({ label: '', url: null })],
            [{ fields: { label: 'Value is required' } }],
            linksField,
        );
        expect(errorOf(getByLabelText('Links, item 1, Label'))).toBe('Value is required');
        expect(errorOf(getByLabelText('Links, item 1, URL (optional)'))).toBeNull();
    });

    test("shows a row's own error under that row's fields", () => {
        const { getByText, getByLabelText } = renderList(
            [listItem({ label: 'Home', url: null }), listItem({ label: 'Home', url: null })],
            [undefined, { error: 'Repeats item 1' }],
            linksField,
        );
        const alert = getByText('Repeats item 1', { selector: '[role="alert"]' });
        expect(alert.parentElement?.contains(getByLabelText('Links, item 2, Label'))).toBe(true);
        expect(alert.parentElement?.contains(getByLabelText('Links, item 1, Label'))).toBe(false);
    });

    test('marks every field of a row its own error is about, each reading that error', () => {
        const { getByLabelText } = renderList(
            [listItem({ label: 'Home', url: null }), listItem({ label: 'Home', url: null })],
            [undefined, { error: 'Repeats item 1' }],
            linksField,
        );
        expect(errorOf(getByLabelText('Links, item 2, Label'))).toBe('Repeats item 1');
        expect(errorOf(getByLabelText('Links, item 2, URL (optional)'))).toBe('Repeats item 1');
        expect(errorOf(getByLabelText('Links, item 1, Label'))).toBeNull();
    });

    test('marks only the fields a row error names', () => {
        const { getByLabelText } = renderList(
            [listItem({ label: 'Home', url: null }), listItem({ label: 'Home', url: 'https://pool' })],
            [undefined, { error: 'Same as item 1', markedFields: ['label'] }],
            linksField,
        );
        expect(errorOf(getByLabelText('Links, item 2, Label'))).toBe('Same as item 1');
        expect(errorOf(getByLabelText('Links, item 2, URL (optional)'))).toBeNull();
    });
});
