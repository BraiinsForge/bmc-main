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

import { afterEach, describe, test, expect } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';
import { MemoryRouter, Route, Routes } from 'react-router';
import { AlarmsCapabilityGate, alarmsRedirectTarget } from './Alarms';
import { URLS } from '@/constants';
import type { Capabilities } from '@/lib/system';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';

afterEach(cleanup);

function renderGate(caps: Capabilities) {
    return render(
        <MemoryRouter initialEntries={[URLS.pages.alarms]}>
            <Routes>
                <Route path={URLS.pages.display.list} element={<div>display list</div>} />
                <Route
                    path={URLS.pages.alarms}
                    element={
                        <AlarmsCapabilityGate capabilities={caps}>
                            <div>alarms body</div>
                        </AlarmsCapabilityGate>
                    }
                />
            </Routes>
        </MemoryRouter>,
    );
}

describe('alarmsRedirectTarget', () => {
    test('null (no redirect) when the backend reports alarm support', () => {
        expect(alarmsRedirectTarget(deckCapabilities({ alarmSupported: true }))).toBeNull();
    });

    test('redirects to the display list without alarm support', () => {
        expect(alarmsRedirectTarget(deckCapabilities({ alarmSupported: false }))).toBe(URLS.pages.display.list);
    });
});

describe('AlarmsCapabilityGate', () => {
    test('renders the page when alarms are supported', () => {
        const view = renderGate(deckCapabilities({ alarmSupported: true }));
        expect(view.getByText('alarms body')).toBeTruthy();
        expect(view.queryByText('display list')).toBeNull();
    });

    test('redirects through the router when alarms are unsupported', async () => {
        const view = renderGate(deckCapabilities({ alarmSupported: false }));
        expect(await view.findByText('display list')).toBeTruthy();
        expect(view.queryByText('alarms body')).toBeNull();
    });
});
