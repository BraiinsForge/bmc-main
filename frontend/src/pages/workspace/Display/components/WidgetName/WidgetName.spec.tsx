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
import { cleanup, render } from '@testing-library/react/pure';
import { WidgetName } from './WidgetName';

beforeEach(cleanup);

describe('WidgetName', () => {
    test('renders the name', () => {
        const { getByText } = render(<WidgetName name="Clock" />);
        expect(getByText('Clock')).toBeTruthy();
    });

    test('renders the subname when present', () => {
        const { getByText } = render(<WidgetName name="Clock" subname="Analog" />);
        expect(getByText('Analog')).toBeTruthy();
    });

    test('omits the subname element when absent', () => {
        const { container } = render(<WidgetName name="Clock" />);
        // Just the name — no trailing subname text.
        expect(container.textContent).toBe('Clock');
    });
});
