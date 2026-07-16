// Copyright (C) 2025  Braiins Systems s.r.o.
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
                <Button id="0" size="md" kind="primary" children="First" icon={IconDashboard} />
                <Button id="1" size="md" kind="secondary" children="Second" icon={IconNotification} />
            </Component>

            <h3 style={titleStyle}>Icon only</h3>
            <Component {...args}>
                <Button id="2" size="md" kind="primary" hasIconOnly icon={IconDashboard} title="xxx" />
                <Button id="3" size="md" kind="secondary" hasIconOnly icon={IconNotification} title="xxx" />
            </Component>

            <h3 style={titleStyle}>Nested</h3>
            <Component {...args}>
                <Button id="4" size="md" kind="primary" children="First" icon={IconDashboard} />
                <Button id="5" size="md" kind="secondary" children="Second" icon={IconNotification} />
                <Component {...args}>
                    <Button id="6" size="md" kind="primary" hasIconOnly icon={IconDashboard} title="xxx" />
                    <Button id="7" size="md" kind="secondary" hasIconOnly icon={IconNotification} title="xxx" />
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
