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

import { cleanup, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { afterEach, describe, expect, test } from '@rstest/core';

import type { UpgradeShape, UpgradeStepperProps } from './UpgradeStepper';
import { UpgradeStepper } from './UpgradeStepper';
import { fixtures } from './UpgradeStepper.fixtures';

const renderStepper = (props: UpgradeStepperProps) =>
    render(
        <IntlProvider locale="en">
            <UpgradeStepper {...props} />
        </IntlProvider>,
    );

// Labels each shape must render, independent of the component's own tables.
const EXPECTED_LABELS: Record<UpgradeShape, string[]> = {
    firmware: ['Downloading', 'Verifying', 'Applying'],
    package: ['Downloading', 'Verifying', 'Building', 'Activating'],
    combined: [
        'Downloading firmware',
        'Verifying firmware',
        'Downloading packages',
        'Verifying packages',
        'Building packages',
        'Applying',
    ],
};

afterEach(cleanup);

describe('UpgradeStepper', () => {
    test.each(Object.entries(fixtures))('%s renders every step of its shape', (_name, props) => {
        renderStepper(props);
        for (const label of EXPECTED_LABELS[props.shape]) {
            expect(screen.getAllByText(label).length).toBeGreaterThan(0);
        }
    });

    test('a combined run shows the package realize download — the step we were dropping', () => {
        renderStepper(fixtures['combined / pkg-realize']);
        expect(screen.getByText('1 MB / 4 MB')).toBeTruthy();
    });

    test('a download step with a known total renders the byte bar', () => {
        renderStepper(fixtures['firmware / downloading']);
        expect(screen.getByText('6 MB / 24 MB')).toBeTruthy();
    });

    test('a non-download step renders the indeterminate spinner, not a byte bar', () => {
        renderStepper(fixtures['combined / pkg-build']);
        expect(screen.getByText(/\d+s elapsed/)).toBeTruthy();
        expect(screen.queryByText(/\d+ .?B \/ \d+ .?B/)).toBeNull();
    });

    test('finalizing shows the finishing indicator and no active-step counter', () => {
        renderStepper(fixtures['package / finalizing']);
        expect(screen.getByText('Finishing up…')).toBeTruthy();
        // Every step is done, so there is no active step ticking an elapsed counter.
        expect(screen.queryByText(/\d+s elapsed/)).toBeNull();
    });
});
