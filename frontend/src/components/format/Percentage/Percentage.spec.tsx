import { beforeEach, describe, test, expect } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';
import { Percentage, type PercentageProps } from './Percentage';

beforeEach(cleanup);

interface Case extends PercentageProps {
    exp: string;
}
const data: ReadonlyArray<Case> = [
    Object.freeze({ value: 2, round: 2, upperValueBound: 1, exp: '200.00' }),
    Object.freeze({ value: 4, round: 2, upperValueBound: 1, exp: '400.00' }),
    Object.freeze({ value: 500, base: -2, upperValueBound: 1, exp: '500.00' }),
    Object.freeze({ value: 500, base: -1, upperValueBound: 100, exp: '50.00' }),
    Object.freeze({ value: 500, base: -3, upperValueBound: 100, exp: '0.50' }),
    // Trim enabled
    Object.freeze({ trim: true, value: 2, round: 2, upperValueBound: 1, exp: '200' }),
    Object.freeze({ trim: true, value: 4, round: 2, upperValueBound: 1, exp: '400' }),
    Object.freeze({ trim: true, value: 500, base: -2, upperValueBound: 1, exp: '500' }),
    Object.freeze({ trim: true, value: 500, base: -1, upperValueBound: 100, exp: '50' }),
];

describe('<Percentage />', () => {
    test.each<Case>(data)('Renders corrent percentage', ({ exp, ...props }) => {
        const { baseElement } = render(<Percentage {...props} />);
        expect(baseElement.querySelector('[data-role="value"]')?.textContent).toEqual(exp);
        expect(baseElement.querySelector('[data-role="unit"]')?.textContent).toEqual('%');
    });
});
