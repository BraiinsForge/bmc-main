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

// Carbon's own English matches most of these defaults,
// so the pseudo-locale is what tells a worded name from Carbon's.

import type { ReactElement } from 'react';
import { afterEach, describe, expect, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { type IntlShape, RawIntlProvider } from 'react-intl';

import { intlFor } from '@/lib/intl';
import { pseudoIntl } from '@/mocks/intl';
import { ComboBox, Dropdown, NumberInput, OverflowMenu, PasswordInput, ProgressStep, Toggle } from '@/components';

afterEach(cleanup);

const english = intlFor({ tag: 'en' });

function renderIn(intl: IntlShape, element: ReactElement) {
    return render(<RawIntlProvider value={intl} children={element} />);
}

describe('ComboBox', () => {
    const combo = <ComboBox id="c" items={['a', 'b']} onChange={() => {}} />;

    test('names itself and its arrow by whether its list is open', () => {
        renderIn(english, combo);
        expect(screen.getByRole('combobox', { name: 'Choose an item' })).toBeTruthy();

        fireEvent.click(screen.getByRole('button', { name: 'Open' }));

        expect(screen.getByRole('button', { name: 'Close' })).toBeTruthy();
    });

    test('takes its names from the catalog', () => {
        renderIn(pseudoIntl, combo);
        expect(screen.queryByRole('combobox', { name: 'Choose an item' })).toBeNull();
        expect(screen.queryByRole('button', { name: 'Open' })).toBeNull();
    });
});

describe('Dropdown', () => {
    const dropdown = <Dropdown id="d" label="Pick one" titleText="Kind" items={['a', 'b']} />;

    test('names its arrow by whether its list is open', () => {
        renderIn(english, dropdown);
        expect(screen.getByTitle('Open')).toBeTruthy();

        fireEvent.click(screen.getByRole('combobox'));

        expect(screen.getByTitle('Close')).toBeTruthy();
    });

    test('takes its arrow name from the catalog', () => {
        renderIn(pseudoIntl, dropdown);
        expect(screen.queryByTitle('Open')).toBeNull();
    });
});

describe('NumberInput', () => {
    const input = <NumberInput id="n" label="Count" value={1} />;

    test('names each step button by its direction', () => {
        renderIn(english, input);
        expect(screen.getByRole('button', { name: 'Increment number' })).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Decrement number' })).toBeTruthy();
    });

    test('takes its button names from the catalog', () => {
        renderIn(pseudoIntl, input);
        expect(screen.queryByRole('button', { name: 'Increment number' })).toBeNull();
        expect(screen.queryByRole('button', { name: 'Decrement number' })).toBeNull();
    });
});

describe('OverflowMenu', () => {
    test('names its icon button', () => {
        renderIn(english, <OverflowMenu />);
        expect(screen.getByRole('button', { name: 'Options' })).toBeTruthy();
    });

    test('takes its name from the catalog', () => {
        renderIn(pseudoIntl, <OverflowMenu />);
        expect(screen.queryByRole('button', { name: 'Options' })).toBeNull();
    });
});

describe('PasswordInput', () => {
    const input = <PasswordInput id="p" labelText="Password" />;

    test('names its button by what pressing it does', () => {
        renderIn(english, input);

        fireEvent.click(screen.getByRole('button', { name: 'Show password' }));

        expect(screen.getByRole('button', { name: 'Hide password' })).toBeTruthy();
    });

    test('takes its button name from the catalog', () => {
        renderIn(pseudoIntl, input);
        expect(screen.queryByRole('button', { name: 'Show password' })).toBeNull();
    });
});

describe('ProgressStep', () => {
    test.each([
        ['complete', { complete: true }, 'Complete'],
        ['current', { current: true }, 'Current'],
        ['incomplete', {}, 'Incomplete'],
        ['invalid', { invalid: true }, 'Invalid'],
    ])('reads a step that is %s as such', (_case, state, name) => {
        renderIn(english, <ProgressStep label="Step" {...state} />);
        expect(screen.getByText(name)).toBeTruthy();
    });

    test('takes its state names from the catalog', () => {
        renderIn(pseudoIntl, <ProgressStep label="Step" complete />);
        expect(screen.queryByText('Complete')).toBeNull();
    });
});

describe('Toggle', () => {
    test('shows its state beside the switch', () => {
        renderIn(english, <Toggle id="t" />);

        fireEvent.click(screen.getByRole('switch'));

        expect(screen.getByText('On')).toBeTruthy();
    });

    test('takes its state names from the catalog', () => {
        renderIn(pseudoIntl, <Toggle id="t" />);
        expect(screen.queryByText('Off')).toBeNull();
    });
});
