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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import { proto } from '@/mocks/proto';
import { BootError } from './BootError';

function renderBootError(error: unknown): void {
    render(
        <IntlProvider locale="en">
            <BootError error={error} />
        </IntlProvider>,
    );
}

afterEach(() => {
    cleanup();
    rstest.restoreAllMocks();
});

describe('BootError', () => {
    test("a lost connection shows the title without the browser's text", () => {
        renderBootError(proto.lostConnection());

        expect(screen.getByText('The device did not describe itself')).toBeTruthy();
        expect(screen.queryByText(/Failed to fetch/)).toBeNull();
    });

    test('a broken device description keeps its diagnostic', () => {
        rstest.spyOn(console, 'error').mockImplementation(() => {});
        renderBootError(new Error('system.js: `window.SYSTEM` is not set'));

        expect(screen.getByText('system.js: `window.SYSTEM` is not set')).toBeTruthy();
    });
});
