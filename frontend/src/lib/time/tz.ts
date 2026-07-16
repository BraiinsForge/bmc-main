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

export interface TimezoneOption {
    value: string;
    label: string;
}

export function formatTimezoneLabel(
    locale: Intl.UnicodeBCP47LocaleIdentifier,
    timeZoneName: string,
    // Usually probably not provided externally,
    // but much more efficient when formatting
    // A LOT of timezones
    __now__: Date = new Date(),
) {
    const formatter = new Intl.DateTimeFormat(locale, {
        timeZone: timeZoneName,
        timeZoneName: 'short',
        hour: '2-digit',
        minute: '2-digit',
    });
    const dateAndOffset = formatter.format(__now__);
    return `${timeZoneName} (${dateAndOffset})`;
}

export function getTimeZonesWithLabels<R = TimezoneOption>(
    locale: Intl.UnicodeBCP47LocaleIdentifier = 'en-UK',
    mapFunction?: (item: TimezoneOption) => R,
): R[] {
    const timeZones = Intl.supportedValuesOf('timeZone');
    const now = new Date();

    return timeZones.map<R>(tzName => {
        const res: TimezoneOption = {
            value: tzName,
            label: formatTimezoneLabel(locale, tzName, now),
        };
        return (mapFunction ? mapFunction(res) : res) as R;
    });
}
