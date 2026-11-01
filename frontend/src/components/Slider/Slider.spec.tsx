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
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import { Slider } from './Slider';

afterEach(cleanup);

describe('<Slider />', () => {
    test('words the alert after it moves a typed value into its range', () => {
        render(
            <IntlProvider locale="en">
                <Slider id="volume" labelText="Volume" min={0} max={100} value={50} />
            </IntlProvider>,
        );

        const field = screen.getByRole('spinbutton');
        fireEvent.change(field, { target: { value: '150' } });
        fireEvent.keyDown(field, { key: 'Enter' });

        expect(screen.getByRole('alert').textContent).toBe(
            'The value “150” was corrected to the nearest allowed value.',
        );
    });
});
