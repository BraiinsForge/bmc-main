// Copyright (C) 2026  Braiins Systems s.r.o.
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
import { cleanup, render } from '@testing-library/react/pure';
import { LinksBar } from './LinksBar';

afterEach(cleanup);

describe('LinksBar', () => {
    test('renders the links a brand file provides', () => {
        const view = render(<LinksBar links={[{ href: 'https://acme.example', text: 'Acme', isActive: true }]} />);
        const link = view.getByText('Acme');

        expect(link.getAttribute('href')).toBe('https://acme.example');
    });

    test('renders nothing without links', () => {
        expect(render(<LinksBar links={[]} />).container.textContent).toBe('');
    });
});
