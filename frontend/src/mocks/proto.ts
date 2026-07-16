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

import * as pb from '@/proto';
import { timestamp } from './time';
import { randomItem } from './collection';
import type { LengthRange } from './generics';

export const proto = {
    randomEnumItem<K extends string, V extends number>(enm: Record<K, V>): V {
        const input = Object.values(enm).filter(x => typeof x === 'number' && x > 0) as V[];
        return randomItem<V>(input);
    },
    timestamp(offset: LengthRange = 0): pb.Timestamp {
        return pb.create(pb.TimestampSchema, { seconds: BigInt(timestamp(offset)) });
    },
};
