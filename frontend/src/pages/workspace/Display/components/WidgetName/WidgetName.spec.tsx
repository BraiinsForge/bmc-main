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
