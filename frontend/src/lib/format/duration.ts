// Copyright (C) 2026  Braiins Systems s.r.o.
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

import type { IntlShape } from 'react-intl';

/**
 * Format whole seconds as minutes and seconds in `intl`'s locale, a zero part left out.
 *
 * In English `90` → `"1 minute, 30 seconds"`, `45` → `"45 seconds"`, `0` → `"0 seconds"`.
 */
export function formatDuration(intl: IntlShape, totalSeconds: number): string {
    const whole = Math.floor(totalSeconds);
    const minutes = Math.floor(whole / 60);
    const seconds = whole % 60;
    const unit = (value: number, unit: 'minute' | 'second') =>
        intl.formatNumber(value, { style: 'unit', unit, unitDisplay: 'long' });

    if (minutes === 0) return unit(seconds, 'second');
    if (seconds === 0) return unit(minutes, 'minute');
    return intl.formatList([unit(minutes, 'minute'), unit(seconds, 'second')], { type: 'unit' });
}
