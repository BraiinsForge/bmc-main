// Copyright (C) 2025  Braiins Systems s.r.o.
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

import type { Meta } from '@storybook/react';
import { WidgetCombined, WifiSignalStrength } from './index';

export default {
    title: 'components/Icons',
} satisfies Meta;

const display = 'flex';
const padding = 8;
const gap = 16;

function Column(props: { children: ReactNode }) {
    return <div style={{ display, flexDirection: 'column', gap, padding, width: 600 }} children={props.children} />;
}
function Row(props: { children: ReactNode }) {
    return <div style={{ display, flexDirection: 'row', gap, padding }} children={props.children} />;
}

export function Icons() {
    return (
        <Column>
            <Row>
                <WifiSignalStrength size={64} state="full" />
                <WifiSignalStrength size={64} state="fair" />
                <WifiSignalStrength size={64} state="low" />
                <WifiSignalStrength size={64} state="offline" />
                <WifiSignalStrength size={64} state="scanning" />
            </Row>
            <Row>
                <WidgetCombined size={64} />
            </Row>
        </Column>
    );
}
