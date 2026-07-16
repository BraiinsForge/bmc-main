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

import { action } from 'storybook/actions';

import { Button } from '@/components';
import { Empty as Component, type EmptyProps } from './Empty';
import { CloudMonitoring, QuestionAnswering } from '@carbon/react/icons';

export default {
    title: 'components/Empty',
    component: Component,
};

export function Empty(args: EmptyProps) {
    return (
        <div style={{ maxWidth: '600px' }}>
            <Component {...args} />
        </div>
    );
}

Empty.args = {
    icon: CloudMonitoring,
    title: 'There are no workers to monitor…',
    message: (
        <span>
            You must first connect workers and then you&apos;ll be able to see a summary here of the events that were
            recorded by our monitoring system.
        </span>
    ),
    controls: (
        <Button id="connect-workers" icon={QuestionAnswering} children="Connect Workers" onClick={action('onClick')} />
    ),
} as EmptyProps;
