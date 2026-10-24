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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import * as pb from '@/proto';
import { SectionGeneral, type SectionGeneralProps } from './SectionGeneral';

function field<T>(value: T) {
    return { value, disabled: false, error: '', onChange: rstest.fn() };
}

const languages = [
    pb.create(pb.LanguageInfoSchema, { tag: 'en-GB', nativeName: 'English (UK)' }),
    pb.create(pb.LanguageInfoSchema, { tag: 'zh', nativeName: '简体中文' }),
];

function renderSection(language: SectionGeneralProps['language']): void {
    render(
        <IntlProvider locale="en">
            <SectionGeneral
                timeFormat={field(pb.TimeFormat.TIME_FORMAT_24_HOUR)}
                timezone={null}
                language={language}
                dateFormat={field(pb.DateFormat.DD_MM_YYYY_DASH)}
                firstWeekDay={field(pb.Weekday.MONDAY)}
                temperatureUnits={field(pb.TemperatureUnit.CELSIUS)}
                unitSystem={field(pb.UnitSystem.METRIC)}
                numberFormat={field(pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL)}
                systemActionsOwned={false}
                onFactoryReset={rstest.fn()}
                onSystemReboot={rstest.fn()}
                onDownloadSupportArchive={rstest.fn()}
            />
        </IntlProvider>,
    );
}

afterEach(cleanup);

describe('SectionGeneral language picker', () => {
    test('a device that does not offer its languages shows no picker', () => {
        renderSection(null);

        expect(screen.queryByText('Language')).toBeNull();
    });

    test('shows the language in effect and sets the one picked', () => {
        const language = { ...field('en-GB'), items: languages };
        renderSection(language);

        fireEvent.click(screen.getByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));

        expect(language.onChange).toHaveBeenCalledWith('zh');
    });

    test('a stored language the device no longer lists selects nothing', () => {
        renderSection({ ...field('cs'), items: languages });

        expect(screen.getByText('Language')).toBeTruthy();
        for (const { nativeName } of languages) {
            expect(screen.queryByText(nativeName)).toBeNull();
        }
    });
});
