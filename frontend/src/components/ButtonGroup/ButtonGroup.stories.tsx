import { ButtonGroup as Component, type ButtonGroupProps } from './ButtonGroup';
import { Button } from '@/components';
import { Dashboard as IconDashboard, Notification as IconNotification } from '@carbon/react/icons';

export default {
    title: 'components/ButtonGroup',
    component: Component,
};

export const ButtonGroup = (args: ButtonGroupProps) => {
    const titleStyle = { color: '#fff', marginTop: 16 };
    return (
        <div>
            <h3 style={titleStyle}>Text + icon</h3>
            <Component {...args}>
                <Button size="md" kind="primary" children="First" icon={IconDashboard} />
                <Button size="md" kind="secondary" children="Second" icon={IconNotification} />
            </Component>

            <h3 style={titleStyle}>Icon only</h3>
            <Component {...args}>
                <Button size="md" kind="primary" hasIconOnly icon={IconDashboard} title="xxx" />
                <Button size="md" kind="secondary" hasIconOnly icon={IconNotification} title="xxx" />
            </Component>

            <h3 style={titleStyle}>Nested</h3>
            <Component {...args}>
                <Button size="md" kind="primary" children="First" icon={IconDashboard} />
                <Button size="md" kind="secondary" children="Second" icon={IconNotification} />
                <Component {...args}>
                    <Button size="md" kind="primary" hasIconOnly icon={IconDashboard} title="xxx" />
                    <Button size="md" kind="secondary" hasIconOnly icon={IconNotification} title="xxx" />
                </Component>
            </Component>
        </div>
    );
};
ButtonGroup.storyName = 'ButtonGroup';
ButtonGroup.args = {
    spaced: false,
    vertical: false,
} as ButtonGroupProps;
