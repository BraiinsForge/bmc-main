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
import { cleanup, render, screen } from '@testing-library/react/pure';
import type { ReactElement } from 'react';
import { IntlProvider } from 'react-intl';

import { Modal, OverflowMenu } from '@/components';
import './carbonFlags';

afterEach(cleanup);

// A component, as the app passes, so the props Carbon injects into the child reach no DOM node.
function MenuPanel() {
    return <button type="button" children="Clone" />;
}

function sentinelsIn(ui: ReactElement): number {
    render(<IntlProvider locale="en" children={ui} />);
    return screen.queryAllByText('Focus sentinel').length;
}

describe('carbonFlags', () => {
    test('a modal traps focus without the English sentinel links', () => {
        const modal = <Modal id="probe" open passiveModal modalHeading="Probe" onRequestClose={() => {}} />;
        expect(sentinelsIn(modal)).toBe(0);
    });

    test('an open overflow menu traps focus without the English sentinel links', () => {
        const menu = <OverflowMenu open children={<MenuPanel />} />;
        expect(sentinelsIn(menu)).toBe(0);
    });
});
