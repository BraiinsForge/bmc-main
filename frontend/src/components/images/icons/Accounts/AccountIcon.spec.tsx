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
import * as pb from '@/proto';
import { AccountIcon } from '.';

beforeEach(cleanup);

const icon = pb.create(pb.IconSchema, { mimeType: 'image/svg+xml', data: 'PHN2Zy8+' });

describe('AccountIcon', () => {
    test('renders the type artwork as a data URL', () => {
        const { container } = render(<AccountIcon icon={icon} size={24} />);
        expect(container.querySelector('img')?.getAttribute('src')).toBe('data:image/svg+xml;base64,PHN2Zy8+');
    });

    test('carries the mime type the backend declared rather than assuming SVG', () => {
        const png = pb.create(pb.IconSchema, { mimeType: 'image/png', data: 'AAAA' });
        const { container } = render(<AccountIcon icon={png} size={24} />);
        expect(container.querySelector('img')?.getAttribute('src')).toBe('data:image/png;base64,AAAA');
    });

    test('falls back to the generic glyph (no img) when the type has no artwork', () => {
        const { container } = render(<AccountIcon size={24} />);
        expect(container.querySelector('img')).toBeNull();
        expect(container.querySelector('svg')).not.toBeNull();
    });

    test('is decorative, so it is not announced beside the name it accompanies', () => {
        const { container } = render(<AccountIcon icon={icon} size={24} />);
        expect(container.querySelector('img')?.getAttribute('alt')).toBe('');
    });
});
