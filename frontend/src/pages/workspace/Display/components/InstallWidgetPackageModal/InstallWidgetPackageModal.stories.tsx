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

import { useEffect } from 'react';
import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import { InstallWidgetPackageModal as Component } from './InstallWidgetPackageModal';

const ID = 'install-widget-package';

function Story({ chosen, installing }: { chosen: boolean; installing: boolean }) {
    // The uploader keeps the chosen file to itself, so a story picks one the way a user does.
    useEffect(() => {
        if (!chosen) return;
        const zone = document.getElementById(`${ID}-file`);
        if (!zone) throw new Error(`BUG: #${ID}-file is rendered`);
        const dataTransfer = new DataTransfer();
        dataTransfer.items.add(new File(['x'.repeat(1500)], 'widget-example.nar'));
        zone.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    }, [chosen]);

    return <Component id={ID} installing={installing} onClose={action('onClose')} onInstall={action('onInstall')} />;
}

export default {
    title: 'Display/Components/InstallWidgetPackageModal',
    component: Component,
} satisfies Meta<typeof Component>;

export const Idle = () => <Story chosen={false} installing={false} />;

export const Chosen = () => <Story chosen installing={false} />;

export const Installing = () => <Story chosen installing />;
