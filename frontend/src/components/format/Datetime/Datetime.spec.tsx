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

import { beforeEach, describe, test, expect } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';

import { Datetime, type DatetimeProps } from './Datetime';

beforeEach(cleanup);

function renderDatetime(props: DatetimeProps) {
    const utils = render(<Datetime {...props} />);

    return {
        ...utils,
        time: () => utils.baseElement.querySelector('span'),
    };
}

const props = {
    value: 1571665269,
    format: '%d.%m.%Y %H:%M:%S',
    placeholder: 'placeholder',
    seconds: false,
    tzname: 'UTC',
};

describe('<Datetime />', () => {
    test('checks if a timestamp is changed into a correct format', () => {
        const { time } = renderDatetime({ ...props });

        expect(time()?.textContent).toEqual('21.10.2019 13:41:09');
    });
});
