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
