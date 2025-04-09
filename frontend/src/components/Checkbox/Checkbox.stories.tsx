import { Checkbox as C, type CheckboxProps } from './Checkbox';

export default {
    title: 'components/Checkbox',
    component: C,
};
export const Checkbox = (args: CheckboxProps) => (
    <div className="ui-box">
        <C {...args} />
    </div>
);
Checkbox.args = {
    id: 'id',
    label: 'Checkbox label',
    description: 'Chicken breasts chili has to have a small, thin lobster component.',
    invalid: false,
    invalidText: '',
} as CheckboxProps;
