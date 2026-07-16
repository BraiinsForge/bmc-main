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

import { Datetime as Component, type DatetimeProps } from './Datetime';

const value = new Date(2017, 1, 15, 10, 20, 40);

export default {
    title: 'components/format/Datetime',
    component: Component,
};

export const Datetime = (args: DatetimeProps) => {
    return (
        <div style={{ fontSize: '2rem', color: '#fff' }}>
            <Component {...args} />
        </div>
    );
};
Datetime.args = {
    value: Math.floor(value.valueOf() / 1e3),
    format: '%d.%m.%Y %H:%M:%S',
    placeholder: '---',
    seconds: false,
    tzname: 'UTC',
} as DatetimeProps;
