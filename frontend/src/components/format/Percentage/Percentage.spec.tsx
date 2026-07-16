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

import { beforeEach, describe, test, expect } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';
import { Percentage, type PercentageProps } from './Percentage';

beforeEach(cleanup);

interface Case extends PercentageProps {
    exp: string;
}
const data: ReadonlyArray<Case> = [
    Object.freeze({ value: 2, round: 2, upperValueBound: 1, exp: '200.00' }),
    Object.freeze({ value: 4, round: 2, upperValueBound: 1, exp: '400.00' }),
    Object.freeze({ value: 500, base: -2, upperValueBound: 1, exp: '500.00' }),
    Object.freeze({ value: 500, base: -1, upperValueBound: 100, exp: '50.00' }),
    Object.freeze({ value: 500, base: -3, upperValueBound: 100, exp: '0.50' }),
    // Trim enabled
    Object.freeze({ trim: true, value: 2, round: 2, upperValueBound: 1, exp: '200' }),
    Object.freeze({ trim: true, value: 4, round: 2, upperValueBound: 1, exp: '400' }),
    Object.freeze({ trim: true, value: 500, base: -2, upperValueBound: 1, exp: '500' }),
    Object.freeze({ trim: true, value: 500, base: -1, upperValueBound: 100, exp: '50' }),
];

describe('<Percentage />', () => {
    test.each<Case>(data)('Renders corrent percentage', ({ exp, ...props }) => {
        const { baseElement } = render(<Percentage {...props} />);
        expect(baseElement.querySelector('[data-role="value"]')?.textContent).toEqual(exp);
        expect(baseElement.querySelector('[data-role="unit"]')?.textContent).toEqual('%');
    });
});
