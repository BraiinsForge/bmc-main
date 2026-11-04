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

import { afterEach, beforeEach, describe, test, expect } from '@rstest/core';
import { act, cleanup, render } from '@testing-library/react/pure';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { store } from '@/store';

import { Date as DateOnly } from './Date';
import { Datetime, type DatetimeProps } from './Datetime';

beforeEach(cleanup);

async function prefer(dateFormat: pb.DateFormat, timeFormat: pb.TimeFormat): Promise<void> {
    const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = {
        getGeneralSettingsData: () => pb.create(pb.GeneralSettingsDataResponseSchema, { dateFormat, timeFormat }),
    };
    mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);
    await store.loadFormats();
}

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
    tzname: 'UTC',
};

describe('<Datetime />', () => {
    test('checks if a timestamp is changed into a correct format', () => {
        const { time } = renderDatetime({ ...props });

        expect(time()?.textContent).toEqual('21.10.2019 13:41:09');
    });
});

describe("<Date /> and <Datetime /> by the device's preferences", () => {
    // 5 August 2025, 14:07 UTC: single digits, so a padded layout differs from an unpadded one.
    const value = Date.UTC(2025, 7, 5, 14, 7) / 1e3;
    const written = (rendered: { baseElement: HTMLElement }) => rendered.baseElement.querySelector('span')?.textContent;

    afterEach(async () => {
        await prefer(pb.DateFormat.DD_MM_YYYY_DOT, pb.TimeFormat.TIME_FORMAT_24_HOUR);
        mocks.clear();
    });

    test.each([
        [pb.DateFormat.DD_MM_YYYY_DOT, '05.08.2025'],
        [pb.DateFormat.DD_MM_YYYY_SLASH, '05/08/2025'],
        [pb.DateFormat.D_M_YYYY_SLASH, '5/8/2025'],
        [pb.DateFormat.M_D_YYYY_SLASH, '8/5/2025'],
        [pb.DateFormat.DD_MM_YYYY_DASH, '05-08-2025'],
        [pb.DateFormat.YYYY_M_D_SLASH, '2025/8/5'],
        [pb.DateFormat.YYYY_MM_DD_DOT, '2025.08.05'],
        [pb.DateFormat.YYYY_MM_DD_DASH, '2025-08-05'],
    ])('date layout %i writes %s', async (dateFormat, expected) => {
        await prefer(dateFormat, pb.TimeFormat.TIME_FORMAT_24_HOUR);

        expect(written(render(<DateOnly value={value} />))).toEqual(expected);
    });

    test.each([
        [pb.TimeFormat.TIME_FORMAT_24_HOUR, '05.08.2025 14:07'],
        [pb.TimeFormat.TIME_FORMAT_12_HOUR, '05.08.2025 02:07 PM'],
    ])('time format %i writes %s', async (timeFormat, expected) => {
        await prefer(pb.DateFormat.DD_MM_YYYY_DOT, timeFormat);

        expect(written(render(<Datetime value={value} />))).toEqual(expected);
    });

    test('a shown date follows a change of the preferences', async () => {
        await prefer(pb.DateFormat.DD_MM_YYYY_DOT, pb.TimeFormat.TIME_FORMAT_24_HOUR);
        const rendered = render(<DateOnly value={value} />);

        await act(() => prefer(pb.DateFormat.YYYY_MM_DD_DASH, pb.TimeFormat.TIME_FORMAT_24_HOUR));

        expect(written(rendered)).toEqual('2025-08-05');
    });

    test('a shown date and time follow a change of the preferences', async () => {
        await prefer(pb.DateFormat.DD_MM_YYYY_DOT, pb.TimeFormat.TIME_FORMAT_24_HOUR);
        const rendered = render(<Datetime value={value} />);

        await act(() => prefer(pb.DateFormat.YYYY_MM_DD_DASH, pb.TimeFormat.TIME_FORMAT_12_HOUR));

        expect(written(rendered)).toEqual('2025-08-05 02:07 PM');
    });

    test('an explicit format wins over them', async () => {
        await prefer(pb.DateFormat.M_D_YYYY_SLASH, pb.TimeFormat.TIME_FORMAT_12_HOUR);

        const { time } = renderDatetime({ value, format: '%Y' });

        expect(time()?.textContent).toEqual('2025');
    });
});
