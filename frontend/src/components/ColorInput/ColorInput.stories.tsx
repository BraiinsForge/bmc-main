import { ColorInput as C, type ColorInputProps } from './ColorInput';

export default {
    title: 'components/ColorInput',
    component: C,
};

export const ColorInput = (args: ColorInputProps) => (
    <div className="ui-box">
        <C {...args} />
    </div>
);
ColorInput.args = {
    id: 'color-input',
    value: '#ff0000',
    labelText: 'Pick a color',
} as ColorInputProps;

export const WithoutLabel = (args: ColorInputProps) => (
    <div className="ui-box">
        <C {...args} />
    </div>
);
WithoutLabel.args = {
    id: 'color-input-no-label',
    value: '#00ff00',
    'aria-label': 'Pick a color',
} as ColorInputProps;

export const Disabled = (args: ColorInputProps) => (
    <div className="ui-box">
        <C {...args} />
    </div>
);
Disabled.args = {
    id: 'color-input-disabled',
    value: '#0000ff',
    labelText: 'Disabled color',
    disabled: true,
} as ColorInputProps;
