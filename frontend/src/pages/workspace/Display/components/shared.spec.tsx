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

import { afterEach, describe, expect, test } from '@rstest/core';
import { cleanup, fireEvent, render } from '@testing-library/react/pure';
import { BoundDropdown, BoundRadioGroup } from './shared';

afterEach(cleanup);

interface Item {
    id: string;
    name: string;
}

const A: Item = { id: 'a', name: 'Pool A' };
const B: Item = { id: 'b', name: 'Pool B' };

const dropdown = (value: Item | null, items: Item[] = [A, B]) => (
    <BoundDropdown<Item>
        id="slot"
        labelText="Account"
        placeholderText="— None —"
        items={items}
        itemToString={item => item?.name ?? ''}
        value={value}
        onChange={() => {}}
    />
);

const toggle = () => document.body.querySelector('#slot');

/// The selection must track the value prop through every transition
/// the editor produces by switching between widgets: another account,
/// no account, and the same account renamed.
describe('BoundDropdown selection follows the value prop', () => {
    test('a bound slot shows its account', () => {
        render(dropdown(A));
        expect(toggle()?.textContent).toContain('Pool A');
    });

    test('rebinding to another account follows', () => {
        const { rerender } = render(dropdown(A));
        rerender(dropdown(B));
        expect(toggle()?.textContent).toContain('Pool B');
        expect(toggle()?.textContent).not.toContain('Pool A');
    });

    test('switching to an unbound value clears the selection', () => {
        const { rerender } = render(dropdown(A));
        rerender(dropdown(null));
        expect(toggle()?.textContent).not.toContain('Pool A');
    });

    test('a renamed account shows its new name', () => {
        const { rerender } = render(dropdown(A));
        const renamed = { ...A, name: 'Pool A2' };
        rerender(dropdown(renamed, [renamed, B]));
        expect(toggle()?.textContent).toContain('Pool A2');
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
