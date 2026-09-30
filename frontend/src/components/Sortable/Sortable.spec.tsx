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

import { afterEach, beforeEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { Sortable } from './Sortable';

const fruits = [
    { id: 1, name: 'Apple' },
    { id: 2, name: 'Banana' },
    { id: 3, name: 'Cherry' },
];
type Fruit = (typeof fruits)[number];

const ROW_HEIGHT = 40;

beforeEach(() => {
    cleanup();
    // jsdom lays nothing out, so every row would collide with the first; stack them as a browser would.
    rstest.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
        const top =
            Math.max(
                0,
                fruits.findIndex(f => f.name === this.textContent),
            ) * ROW_HEIGHT;
        return {
            x: 0,
            y: top,
            left: 0,
            top,
            right: 200,
            bottom: top + ROW_HEIGHT,
            width: 200,
            height: ROW_HEIGHT,
            toJSON: () => ({}),
        };
    });
});
afterEach(() => {
    rstest.restoreAllMocks();
});

function renderFruits(onChange: (items: Fruit[]) => void = () => {}) {
    return render(
        <IntlProvider locale="en">
            <Sortable
                items={fruits}
                onChange={onChange}
                getItemLabel={fruit => fruit.name}
                renderItem={({ item, rootProps, dragHandleProps }) => (
                    <div {...rootProps}>
                        <div {...dragHandleProps} children={item.name} />
                    </div>
                )}
            />
        </IntlProvider>,
    );
}

function announcement(): string | undefined {
    return document.querySelector('[id^="DndLiveRegion"]')?.textContent ?? undefined;
}

/** Waits for the pick-up: the sensor only listens for the next key a tick after it activates. */
async function pickUp(handle: HTMLElement, expected: string) {
    handle.focus();
    fireEvent.keyDown(handle, { code: 'Space' });
    await waitFor(() => expect(announcement()).toBe(expected));
}

describe('Sortable announcements', () => {
    test('name the picked-up item and the position it holds', async () => {
        const { getByText } = renderFruits();
        await pickUp(getByText('Banana'), 'Picked up Banana, position 2 of 3.');
    });

    test('follow a keyboard move to the position the item lands in', async () => {
        const onChange = rstest.fn<(items: Fruit[]) => void>();
        const { getByText } = renderFruits(onChange);
        const handle = getByText('Banana');
        await pickUp(handle, 'Picked up Banana, position 2 of 3.');

        fireEvent.keyDown(handle, { code: 'ArrowDown' });
        await waitFor(() => expect(announcement()).toBe('Banana moved to position 3 of 3.'));

        fireEvent.keyDown(handle, { code: 'Space' });
        await waitFor(() => expect(announcement()).toBe('Banana dropped at position 3 of 3.'));
        expect(onChange.mock.calls[0]?.[0].map(f => f.name)).toEqual(['Apple', 'Cherry', 'Banana']);
    });

    test('put a cancelled item back at its own position', async () => {
        const { getByText } = renderFruits();
        const handle = getByText('Banana');
        await pickUp(handle, 'Picked up Banana, position 2 of 3.');

        fireEvent.keyDown(handle, { code: 'Escape' });
        await waitFor(() => expect(announcement()).toBe('Move cancelled, Banana is back at position 2 of 3.'));
    });
});
