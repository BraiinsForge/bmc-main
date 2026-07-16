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

import type { Meta } from '@storybook/react';
import { Progressbar } from '../Progressbar';
import * as gen from '@/mocks';

import { Tick as Component, type TickProps } from './Tick';

export default {
    title: 'components/Tick',
    component: Component,
    args: {
        intervalMs: 1e3,
    } satisfies TickProps,
} satisfies Meta<TickProps>;

const startTime = gen.timestamp(0);

export function Tick(args: TickProps) {
    return (
        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 16 }}>
            <Component {...args} render={value => <div children={value} />} />
            <Component {...args} render={value => <Progressbar values={[{ value: value - startTime }]} />} />
        </div>
    );
}
