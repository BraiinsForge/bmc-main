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
import { ScenePreview } from '.';

beforeEach(cleanup);

const manifest = (iconUrl?: string) => pb.create(pb.WidgetManifestSchema, { uid: 'w', name: 'W', iconUrl });

describe('ScenePreview', () => {
    test('renders the manifest icon when iconUrl is present', () => {
        const { container } = render(<ScenePreview kind={{ manifest: manifest('/widgets/w/icon') }} />);
        expect(container.querySelector('img')?.getAttribute('src')).toBe('/widgets/w/icon');
    });

    test('falls back to the generic glyph (no img) when iconUrl is absent', () => {
        const { container } = render(<ScenePreview kind={{ manifest: manifest() }} />);
        expect(container.querySelector('img')).toBeNull();
    });

    test('combined scenes render the generic glyph, not an img', () => {
        const { container } = render(<ScenePreview kind="combined" />);
        expect(container.querySelector('img')).toBeNull();
    });
});
