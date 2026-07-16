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

import type { Timestamp } from '@bufbuild/protobuf/wkt';
import { type LengthRange, getLength } from './generics';
import { getTimestamp } from '@/lib/time';

const SECONDS = {
    second: 1,
    minute: 60,
    hour: 3_600,
    day: 86_400,
    year: 31_556_926,
};
const ts = (offset: LengthRange = 0): number => getTimestamp(getLength(offset));
export const timestamp = Object.assign(ts, {
    minutes: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.minute),
    hours: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.hour),
    days: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.day),
    months: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.day * 31),
});

function protoTs(offset: LengthRange = 0): Timestamp {
    const seconds = BigInt(getTimestamp(getLength(offset)));
    return { $typeName: 'google.protobuf.Timestamp', seconds, nanos: 0 };
}
export const protoTimestamp = Object.assign(protoTs, {
    minutes: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.minute),
    hours: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.hour),
    days: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.day),
    months: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.day * 31),
});
