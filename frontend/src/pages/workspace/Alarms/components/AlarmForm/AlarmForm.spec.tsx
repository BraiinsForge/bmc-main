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

import { afterEach, describe, expect, test } from '@rstest/core';
import { cleanup, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import * as pb from '@/proto';

import { AlarmForm } from './AlarmForm';

const noop = () => {};

function renderForm(timeFormat = pb.TimeFormat.TIME_FORMAT_24_HOUR) {
    render(
        <IntlProvider locale="en">
            <AlarmForm
                time={{ value: '11:55', onChange: noop }}
                timeFormat={timeFormat}
                name={{ value: '', onChange: noop }}
                repeat={{ value: [], onChange: noop }}
                sound={{ value: null, options: [], onChange: noop }}
                snoozeEnabled={{ value: false, onChange: noop }}
                snoozeLimit={{ value: pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER, onChange: noop }}
                snoozeDuration={{ value: pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES, onChange: noop }}
            />
        </IntlProvider>,
    );
}

describe('<AlarmForm /> on a 12-hour clock', () => {
    afterEach(cleanup);

    test('names the select that picks AM or PM', () => {
        renderForm(pb.TimeFormat.TIME_FORMAT_12_HOUR);

        expect(screen.getByRole('combobox', { name: 'AM or PM' })).toBeTruthy();
    });
});
