import { beforeEach, describe, test, expect, rstest } from '@rstest/core';
import { cleanup, render, fireEvent } from '@testing-library/react/pure';
import { ColorInput } from './ColorInput';

beforeEach(cleanup);

describe('<ColorInput />', () => {
    test('renders with label when labelText is provided', () => {
        const { container } = render(<ColorInput id="test" value="#ff0000" labelText="Pick a color" />);

        const label = container.querySelector('label');
        expect(label).not.toBeNull();
        expect(label?.textContent).toBe('Pick a color');
        expect(label?.getAttribute('for')).toBe('test');
    });

    test('renders without label when labelText is not provided', () => {
        const { container } = render(<ColorInput id="test" value="#ff0000" />);

        const label = container.querySelector('label');
        expect(label).toBeNull();
    });

    test('uses aria-label when provided', () => {
        const { container } = render(<ColorInput id="test" value="#ff0000" aria-label="Color picker" />);

        const input = container.querySelector('input');
        expect(input?.getAttribute('aria-label')).toBe('Color picker');
    });

    test('calls onChange with hex value and RGB when color changes', () => {
        const handleChange = rstest.fn();
        const { container } = render(<ColorInput id="test" value="#000000" onChange={handleChange} />);

        const input = container.querySelector('input');
        if (input) fireEvent.change(input, { target: { value: '#ff8040' } });

        expect(handleChange).toHaveBeenCalledTimes(1);
        expect(handleChange).toHaveBeenCalledWith('#ff8040', { r: 255, g: 128, b: 64 });
    });

    test('defaults to #000000 when value is empty', () => {
        const { container } = render(<ColorInput id="test" value="" />);

        const input = container.querySelector('input');
        expect(input?.value).toBe('#000000');
    });

    test('renders as disabled when disabled prop is true', () => {
        const { container } = render(<ColorInput id="test" value="#ff0000" disabled />);

        const input = container.querySelector('input');
        expect(input?.disabled).toBe(true);
    });

    test('applies custom className and style', () => {
        const { container } = render(
            <ColorInput id="test" value="#ff0000" className="custom-class" style={{ marginTop: 10 }} />,
        );

        const root = container.firstElementChild;
        expect(root?.classList.contains('custom-class')).toBe(true);
        expect((root as HTMLElement)?.style.marginTop).toBe('10px');
    });
});

describe('hexToRgb conversion', () => {
    const cases = [
        { hex: '#000000', rgb: { r: 0, g: 0, b: 0 } },
        { hex: '#ffffff', rgb: { r: 255, g: 255, b: 255 } },
        { hex: '#ff0000', rgb: { r: 255, g: 0, b: 0 } },
        { hex: '#00ff00', rgb: { r: 0, g: 255, b: 0 } },
        { hex: '#0000ff', rgb: { r: 0, g: 0, b: 255 } },
        { hex: '#123456', rgb: { r: 18, g: 52, b: 86 } },
        { hex: '#abcdef', rgb: { r: 171, g: 205, b: 239 } },
    ];

    test.each(cases)('converts $hex to RGB correctly', ({ hex, rgb }) => {
        const handleChange = rstest.fn();
        // Use a different initial value to ensure the change event fires
        const { container } = render(<ColorInput id="test" value="#999999" onChange={handleChange} />);

        const input = container.querySelector('input');
        if (input) fireEvent.change(input, { target: { value: hex } });

        expect(handleChange).toHaveBeenCalledWith(hex, rgb);
    });
});
