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
import type { ReadonlyDeep, SetNonNullable } from 'type-fest';

export function pos(row: number, col: number): pb.WidgetPosition {
    return pb.create(pb.WidgetPositionSchema, { row, col });
}

export function placeholder(row: number, col: number): WidgetPlaceholder {
    return {
        id: `placeholder-${row}-${col}`,
        isPlaceholder: true,
        position: pos(row, col),
        size: pb.WidgetSize.SMALL,
    };
}

type FullyFormed<T> = Required<SetNonNullable<T>>;
export type WidgetPlaceholder = FullyFormed<Pick<pb.Widget, 'id' | 'size' | 'position'> & { isPlaceholder: true }>;
export type WidgetOrPlaceholder = pb.Widget | WidgetPlaceholder;
export type WidgetsWithPlaceholders = WidgetOrPlaceholder[];
export type WidgetsOccupandyMap = [
    row1: [boolean, boolean, boolean, boolean],
    row2: [boolean, boolean, boolean, boolean],
];

export const EMPTY_WIDGETS_OCCUPANDY_MAP: Readonly<WidgetsOccupandyMap> = Object.freeze([
    [false, false, false, false],
    [false, false, false, false],
]);

export const WIDGET_SIZE_TO_SPAN: ReadonlyDeep<ProtoEnumRecord<pb.WidgetSize, { rows: number; cols: number }>> =
    Object.freeze({
        [pb.WidgetSize.SMALL]: Object.freeze({ rows: 1, cols: 1 }),
        [pb.WidgetSize.MEDIUM]: Object.freeze({ rows: 1, cols: 2 }),
        [pb.WidgetSize.LARGE]: Object.freeze({ rows: 2, cols: 2 }),
        [pb.WidgetSize.FULL]: Object.freeze({ rows: 2, cols: 4 }),
    });

export type Located = Pick<pb.Widget, 'id' | 'position' | 'size'>;

export type DropSlotsKey = `R${number};C${number}`;
export type ValidDropSlots = Set<DropSlotsKey>;
export const dropSlotKey = (row: number, col: number): DropSlotsKey => `R${row};C${col}`;

export type Size = Exclude<pb.WidgetSize, 0>;
export type Position = Exclude<pb.WidgetPosition, 0>;
export type MaybePosition = null | pb.WidgetPosition;
