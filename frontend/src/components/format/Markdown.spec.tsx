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
import { cleanup, render as r } from '@testing-library/react/pure';
import { Markdown } from './Markdown';

beforeEach(cleanup);

const render = (source?: null | string) => r(<Markdown source={source} />).container;

describe('Markdown', () => {
    test('renders inline markup as elements', () => {
        expect(render('**bold**').querySelector('strong')?.textContent).toBe('bold');
    });

    test('turns a bare URL into a link', () => {
        const link = render('see https://example.com').querySelector('a');

        expect(link?.getAttribute('href')).toBe('https://example.com');
    });

    test('leaves raw HTML in the source inert', () => {
        // `Html` sanitizes with `<b>` on its allowlist, so an element here
        // would mean the renderer emitted it — this pins `html: false`.
        expect(render('a <b>bold</b> claim').querySelector('b')).toBeNull();
    });

    test('does not turn a single newline into a line break', () => {
        expect(render('one\ntwo').querySelector('br')).toBeNull();
    });

    test('renders nothing for an absent source', () => {
        expect(render(null).textContent).toBe('');
    });
});

// The source is not ours: widget manifests supply the descriptions this renders,
// and a firmware release note reaches it too.
describe('Markdown against a hostile source', () => {
    test('emits no script element', () => {
        expect(render('<script>alert(1)</script>').querySelector('script')).toBeNull();
    });

    // Control for the two refusals below, which pass on an absent anchor:
    // without this, a broken link parser would read as a working defence.
    test('renders a link with an ordinary scheme', () => {
        const link = render('[click me](https://example.com)').querySelector('a');

        expect(link?.getAttribute('href')).toBe('https://example.com');
    });

    test('refuses a javascript: link', () => {
        const root = render('[click me](javascript:alert(1))');

        expect(root.querySelector('a[href^="javascript:"]')).toBeNull();
    });

    test('refuses a data: link', () => {
        const root = render('[click me](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)');

        expect(root.querySelector('a[href^="data:"]')).toBeNull();
    });

    test('strips an event handler off raw markup', () => {
        const root = render('<img src="x" onerror="alert(1)">');

        expect(root.querySelector('[onerror]')).toBeNull();
    });

    test('strips an event handler smuggled through a link title', () => {
        const root = render('[click me](https://example.com "x\\" onmouseover=\\"alert(1)")');

        expect(root.querySelector('[onmouseover]')).toBeNull();
    });
});
