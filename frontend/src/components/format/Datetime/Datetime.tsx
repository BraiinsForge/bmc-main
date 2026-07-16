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

import { localTimeFormat } from '@/lib/time';
import type { Timestamp as PbTimestamp } from '@/proto';

const FAULT_MARKER = `invalid-datetime-value-${Math.random()}`;

export interface DatetimeProps {
    value: Maybe<Timestamp | PbTimestamp | bigint | Date>; // Posix timestamp (seconds)

    format?: Maybe<string>; // d3's format string
    seconds?: boolean; // determines if the default format string (used if none supplied) will include seconds
    tzname?: string; // Timezone name

    placeholder?: ReactNode;
    className?: string;
    style?: CSSProperties;
}
export function Datetime(props: DatetimeProps) {
    const { value, format, placeholder = '---', seconds, tzname, className, ...rest } = props;
    const $format = format || (seconds ? '%d.%m.%Y %H:%M:%S' : '%d.%m.%Y %H:%M');

    let res: ReactNode = placeholder;
    if (value != null) res = localTimeFormat(value, $format, tzname, FAULT_MARKER);
    if (res === FAULT_MARKER) res = placeholder;

    return <span {...rest} className={className} children={res} />;
}
