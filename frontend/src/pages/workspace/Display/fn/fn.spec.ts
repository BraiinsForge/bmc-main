// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { describe, test, expect } from '@rstest/core';

import * as pb from '@/proto';
import * as C from './const';
import {
    mapOccupiedSlots,
    injectPlaceholdersToUnoccupiedSlots,
    fitsRightDown,
    fitsRightAbove,
    fitsLeftDown,
    fitsLeftAbove,
    explodeWidgetIntoAtoms,
    getValidDropSlots,
    defaultFormifiedValue,
    widgetParamsToFormifiedState,
    buildFieldValues as buildFieldValuesIn,
    fitStoredParams as fitStoredParamsIn,
    hasFieldErrors,
    itemErrorsOf,
    clearFieldError,
    formNotices,
    saveNotices,
    unplacedErrors,
    withLocalErrors,
    clearSlotError,
    clearGlobalErrors,
    revalidateField as revalidateFieldIn,
    type ParamsFormErrors,
    credentialBindingsValid,
    credentialBindingsFor,
    withoutDeletedAccounts,
    explainTimeout,
} from './fn';
import { Code, ConnectError } from '@connectrpc/connect';
import { fakeIntlProp, pseudoIntl } from '@/mocks/intl';
import { listItem, type ListItem, type FieldValue } from '@/components/ParamField/value';
import { parseFormifiedValue as parseFormifiedValueIn } from '@/components/ParamField/parse';
import { paramDef } from './test-helpers';

const parseFormifiedValue = (def: pb.ManifestParamDefinition, raw: FieldValue) =>
    parseFormifiedValueIn(fakeIntlProp, def, raw);
const buildFieldValues = (manifest: pb.WidgetManifest, params: Parameters<typeof buildFieldValuesIn>[2]) =>
    buildFieldValuesIn(fakeIntlProp, manifest, params);
const fitStoredParams = (manifest: pb.WidgetManifest, stored: pb.FieldValues) =>
    fitStoredParamsIn(fakeIntlProp, manifest, stored);
const revalidateField = (
    errors: null | ParamsFormErrors,
    def: pb.ManifestParamDefinition,
    value: Parameters<typeof revalidateFieldIn>[3],
) => revalidateFieldIn(fakeIntlProp, errors, def, value);

const emptyParams = pb.create(pb.FieldValuesSchema, { fields: {} });

const testOneInp: pb.Widget[] = [
    {
        $typeName: 'braiins.bmc.web.Widget',
        config: { $typeName: 'braiins.bmc.web.WidgetConfig', widgetUid: '', params: emptyParams },
        id: '1',
        position: C.pos(0, 0),
        size: pb.WidgetSize.LARGE,
    },
    {
        $typeName: 'braiins.bmc.web.Widget',
        config: { $typeName: 'braiins.bmc.web.WidgetConfig', widgetUid: '', params: emptyParams },
        id: '2',
        position: C.pos(0, 2),
        size: pb.WidgetSize.SMALL,
    },
    {
        $typeName: 'braiins.bmc.web.Widget',
        config: { $typeName: 'braiins.bmc.web.WidgetConfig', widgetUid: '', params: emptyParams },
        id: '3',
        position: C.pos(1, 2),
        size: pb.WidgetSize.MEDIUM,
    },
];
const testOneOut: C.WidgetsOccupandyMap = [
    [true, true, true, false],
    [true, true, true, true],
];

const testTwoInp: pb.Widget[] = [
    {
        $typeName: 'braiins.bmc.web.Widget',
        config: { $typeName: 'braiins.bmc.web.WidgetConfig', widgetUid: '', params: emptyParams },
        id: '1',
        position: C.pos(0, 0),
        size: pb.WidgetSize.LARGE,
    },
];
const testTwoOut: C.WidgetsOccupandyMap = [
    [true, true, false, false],
    [true, true, false, false],
];

const testFullInp: pb.Widget[] = [
    C.pos(0, 0),
    C.pos(0, 1),
    C.pos(0, 2),
    C.pos(0, 3),
    C.pos(1, 0),
    C.pos(1, 1),
    C.pos(1, 2),
    C.pos(1, 3),
].map((position, index) => ({
    $typeName: 'braiins.bmc.web.Widget',
    config: { $typeName: 'braiins.bmc.web.WidgetConfig', widgetUid: '', params: emptyParams },
    id: String(index),
    position,
    size: pb.WidgetSize.SMALL,
}));
const testFullOut = [
    [true, true, true, true],
    [true, true, true, true],
];

describe('mapOccupiedSlots', () => {
    describe('returns correct occupied slots', () => {
        test('test 1', () => {
            expect(mapOccupiedSlots(testOneInp)).toEqual(testOneOut);
        });

        test('test 2', () => {
            expect(mapOccupiedSlots(testTwoInp)).toEqual(testTwoOut);
        });

        test('test full', () => {
            expect(mapOccupiedSlots(testFullInp)).toEqual(testFullOut);
        });
    });
});

describe('injectPlaceholdersToUnoccupiedSlots', () => {
    describe('injects placeholders to unoccupied slots', () => {
        test('test 1', () => {
            const out = injectPlaceholdersToUnoccupiedSlots(testOneInp);
            expect(mapOccupiedSlots(out)).toEqual(testFullOut);
        });

        test('test 2', () => {
            const out = injectPlaceholdersToUnoccupiedSlots(testTwoInp);
            expect(mapOccupiedSlots(out)).toEqual(testFullOut);
        });

        test('test full', () => {
            const out = injectPlaceholdersToUnoccupiedSlots(testFullInp);
            expect(mapOccupiedSlots(out)).toEqual(testFullOut);
        });
    });
});

describe('Direction fitting functions', () => {
    // Test map with some occupied slots:
    // ┌─────────┐
    // │ ■ □ □ □ │ (row 0)
    // │ □ ■ □ □ │ (row 1)
    // └─────────┘
    //   0 1 2 3 (cols)
    // ■ = occupied (true)
    // □ = empty (false)
    const testMap: C.WidgetsOccupandyMap = [
        [true, false, false, false],
        [false, true, false, false],
    ];

    // Empty map for easier testing:
    // ┌─────────┐
    // │ □ □ □ □ │ (row 0)
    // │ □ □ □ □ │ (row 1)
    // └─────────┘
    //   0 1 2 3 (cols)
    const emptyMap: C.WidgetsOccupandyMap = [
        [false, false, false, false],
        [false, false, false, false],
    ];

    describe('fitsRightDown', () => {
        test('SMALL widget fits in empty space', () => {
            // Empty map: place 1×1 at (0,0) → fits
            // ┌─────────┐
            // │ ▼ □ □ □ │ ← widget placed here
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsRightDown(emptyMap, C.pos(0, 0), pb.WidgetSize.SMALL)).toEqual(C.pos(0, 0));

            // Test map: place 1×1 at (0,1) → fits
            // ┌─────────┐
            // │ ■ ▼ □ □ │ ← widget placed here
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(0, 1), pb.WidgetSize.SMALL)).toEqual(C.pos(0, 1));
        });

        test('MEDIUM widget fits when there is horizontal space', () => {
            // Empty map: place 1×2 at (0,0) → fits
            // ┌─────────┐
            // │ ▼─▶ □ □ │ ← widget spans right
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsRightDown(emptyMap, C.pos(0, 0), pb.WidgetSize.MEDIUM)).toEqual(C.pos(0, 0));

            // Test map: place 1×2 at (0,2) → fits
            // ┌─────────┐
            // │ ■ □ ▼─▶ │ ← widget spans right
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(0, 2), pb.WidgetSize.MEDIUM)).toEqual(C.pos(0, 2));
        });

        test('LARGE widget fits when there is 2x2 space', () => {
            // Empty map: place 2×2 at (0,0) → fits
            // ┌─────────┐
            // │ ▼─▶ □ □ │ ← widget spans right & down
            // │ ▼─▶ □ □ │ ←
            // └─────────┘
            expect(fitsRightDown(emptyMap, C.pos(0, 0), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 0));

            // Test map: place 2×2 at (0,2) → fits
            // ┌─────────┐
            // │ ■ □ ▼─▶ │ ← widget spans right & down
            // │ □ ■ ▼─▶ │ ←
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(0, 2), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 2));
        });

        test('FULL widget fits when there is 2x4 space', () => {
            // Empty map: place 2×4 at (0,0) → fits (spans entire map)
            // ┌─────────┐
            // │ ▼─▶─▶─▶ │ ← widget spans entire width
            // │ ▼─▶─▶─▶ │ ← and 2 rows high
            // └─────────┘
            expect(fitsRightDown(emptyMap, C.pos(0, 0), pb.WidgetSize.FULL)).toEqual(C.pos(0, 0));
        });

        test('widget does not fit when space is occupied', () => {
            // Test map: try to place 1×1 at (0,0) → blocked by occupied slot
            // ┌─────────┐
            // │ ✗ □ □ □ │ ← blocked by ■
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(0, 0), pb.WidgetSize.SMALL)).toBe(null);

            // Test map: try to place 1×1 at (1,1) → blocked by occupied slot
            // ┌─────────┐
            // │ ■ □ □ □ │
            // │ □ ✗ □ □ │ ← blocked by ■
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(1, 1), pb.WidgetSize.SMALL)).toBe(null);
        });

        test('widget does not fit when there is insufficient space', () => {
            // Test map: try to place 1×2 at (0,3) → would exceed bounds
            // ┌─────────┐
            // │ ■ □ □ ✗ │ ← would need col 4 (out of bounds)
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(0, 3), pb.WidgetSize.MEDIUM)).toBe(null);

            // Test map: try to place 2×2 at (1,3) → would exceed bounds
            // ┌─────────┐
            // │ ■ □ □ □ │
            // │ □ ■ □ ✗ │ ← would need col 4 & row 2 (out of bounds)
            // └─────────┘
            expect(fitsRightDown(testMap, C.pos(1, 3), pb.WidgetSize.LARGE)).toBe(null);
        });
    });

    describe('fitsRightUp', () => {
        test('SMALL widget fits in empty space', () => {
            // Empty map: place 1×1 at (1,0) → fits
            // ┌─────────┐
            // │ □ □ □ □ │
            // │ ▲ □ □ □ │ ← widget placed here (from pos up)
            // └─────────┘
            expect(fitsRightAbove(emptyMap, C.pos(1, 0), pb.WidgetSize.SMALL)).toEqual(C.pos(1, 0));

            // Test map: place 1×1 at (1,2) → fits
            // ┌─────────┐
            // │ ■ □ □ □ │
            // │ □ ■ ▲ □ │ ← widget placed here (from pos up)
            // └─────────┘
            expect(fitsRightAbove(testMap, C.pos(1, 2), pb.WidgetSize.SMALL)).toEqual(C.pos(1, 2));
        });

        test('MEDIUM widget fits when there is horizontal space upward', () => {
            // Empty map: place 1×2 at (1,0) → fits (spans right from pos, up 1 row)
            // ┌─────────┐
            // │ ◄─▲ □ □ │ ← widget occupies this row
            // │ ▲─▶ □ □ │ ← placed from this position
            // └─────────┘
            expect(fitsRightAbove(emptyMap, C.pos(1, 0), pb.WidgetSize.MEDIUM)).toEqual(C.pos(1, 0));

            // Test map: place 1×2 at (1,2) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▲ │ ← widget occupies this row
            // │ □ ■ ▲─▶ │ ← placed from this position
            // └─────────┘
            expect(fitsRightAbove(testMap, C.pos(1, 2), pb.WidgetSize.MEDIUM)).toEqual(C.pos(1, 2));
        });

        test('LARGE widget fits when there is 2x2 space upward', () => {
            // Empty map: place 2×2 at (1,0) → fits (spans right & up from pos)
            // ┌─────────┐
            // │ ◄─▲ □ □ │ ← widget occupies this area
            // │ ▲─▶ □ □ │ ← placed from this position
            // └─────────┘
            expect(fitsRightAbove(emptyMap, C.pos(1, 0), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 0));

            // Test map: place 2×2 at (1,2) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▲ │ ← widget occupies this area
            // │ □ ■ ▲─▶ │ ← placed from this position
            // └─────────┘
            expect(fitsRightAbove(testMap, C.pos(1, 2), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 2));
        });

        test('widget does not fit when position would go out of bounds', () => {
            // Empty map: try to place 2×2 at (0,0) → would need row -1 (out of bounds)
            // ┌─────────┐ ← would need to extend above this
            // │ ✗ □ □ □ │ ← trying to place from here
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsRightAbove(emptyMap, C.pos(0, 0), pb.WidgetSize.LARGE)).toBe(null);
        });

        test('widget does not fit when space is occupied', () => {
            // Test map: try to place 1×1 at (1,1) → blocked by occupied slot
            // ┌─────────┐
            // │ ■ □ □ □ │
            // │ □ ✗ □ □ │ ← trying to place from here (blocked by ■)
            // └─────────┘
            expect(fitsRightAbove(testMap, C.pos(1, 1), pb.WidgetSize.SMALL)).toBe(null);
        });
    });

    describe('fitsLeftDown', () => {
        test('SMALL widget fits in empty space', () => {
            // Empty map: place 1×1 at (0,1) → fits
            // ┌─────────┐
            // │ □ ▼ □ □ │ ← widget placed here
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsLeftDown(emptyMap, C.pos(0, 1), pb.WidgetSize.SMALL)).toEqual(C.pos(0, 1));

            // Test map: place 1×1 at (0,2) → fits
            // ┌─────────┐
            // │ ■ □ ▼ □ │ ← widget placed here
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsLeftDown(testMap, C.pos(0, 2), pb.WidgetSize.SMALL)).toEqual(C.pos(0, 2));
        });

        test('MEDIUM widget fits when there is horizontal space leftward', () => {
            // Empty map: place 1×2 at (0,1) → fits (spans left from pos)
            // ┌─────────┐
            // │ ◄─▼ □ □ │ ← widget spans left from position
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsLeftDown(emptyMap, C.pos(0, 1), pb.WidgetSize.MEDIUM)).toEqual(C.pos(0, 0));

            // Test map: place 1×2 at (0,3) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▼ │ ← widget spans left from position
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsLeftDown(testMap, C.pos(0, 3), pb.WidgetSize.MEDIUM)).toEqual(C.pos(0, 2));
        });

        test('LARGE widget fits when there is 2x2 space leftward', () => {
            // Empty map: place 2×2 at (0,1) → fits (spans left & down from pos)
            // ┌─────────┐
            // │ ◄─▼ □ □ │ ← widget spans left & down
            // │ ◄─▼ □ □ │ ←
            // └─────────┘
            expect(fitsLeftDown(emptyMap, C.pos(0, 1), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 0));

            // Test map: place 2×2 at (0,3) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▼ │ ← widget spans left & down
            // │ □ ■ ◄─▼ │ ←
            // └─────────┘
            expect(fitsLeftDown(testMap, C.pos(0, 3), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 2));
        });

        test('widget does not fit when position would go out of bounds', () => {
            // Empty map: try to place 1×2 at (0,0) → would need col -1 (out of bounds)
            //             ↙ would need to extend left of this
            // ┌─────────┐
            // │ ✗ □ □ □ │ ← trying to place from here
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsLeftDown(emptyMap, C.pos(0, 0), pb.WidgetSize.MEDIUM)).toBe(null);
        });

        test('widget does not fit when space is occupied', () => {
            // Test map: try to place 1×2 at (0,1) → would collide with occupied slot
            // ┌─────────┐
            // │ ■ ✗ □ □ │ ← would collide with ■ to the left
            // │ □ ■ □ □ │
            // └─────────┘
            expect(fitsLeftDown(testMap, C.pos(0, 1), pb.WidgetSize.MEDIUM)).toBe(null);
        });
    });

    describe('fitsLeftAbove', () => {
        test('SMALL widget fits in empty space', () => {
            // Empty map: place 1×1 at (1,1) → fits
            // ┌─────────┐
            // │ □ □ □ □ │
            // │ □ ▲ □ □ │ ← widget placed here (from pos left & up)
            // └─────────┘
            expect(fitsLeftAbove(emptyMap, C.pos(1, 1), pb.WidgetSize.SMALL)).toEqual(C.pos(1, 1));

            // Test map: place 1×1 at (1,2) → fits
            // ┌─────────┐
            // │ ■ □ □ □ │
            // │ □ ■ ▲ □ │ ← widget placed here (from pos left & up)
            // └─────────┘
            expect(fitsLeftAbove(testMap, C.pos(1, 2), pb.WidgetSize.SMALL)).toEqual(C.pos(1, 2));
        });

        test('MEDIUM widget fits when there is horizontal space leftward and upward', () => {
            // Empty map: place 1×2 at (1,1) → fits (spans left & up from pos)
            // ┌─────────┐
            // │ ◄─▲ □ □ │ ← widget occupies this row
            // │ ◄─▲ □ □ │ ← placed from this position
            // └─────────┘
            expect(fitsLeftAbove(emptyMap, C.pos(1, 1), pb.WidgetSize.MEDIUM)).toEqual(C.pos(1, 0));

            // Test map: place 1×2 at (1,3) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▲ │ ← widget occupies this area
            // │ □ ■ ◄─▲ │ ← placed from this position
            // └─────────┘
            expect(fitsLeftAbove(testMap, C.pos(1, 3), pb.WidgetSize.MEDIUM)).toEqual(C.pos(1, 2));
        });

        test('LARGE widget fits when there is 2x2 space leftward and upward', () => {
            // Empty map: place 2×2 at (1,1) → fits (spans left & up from pos)
            // ┌─────────┐
            // │ ◄─▲ □ □ │ ← widget occupies this area
            // │ ◄─▲ □ □ │ ← placed from this position
            // └─────────┘
            expect(fitsLeftAbove(emptyMap, C.pos(1, 1), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 0));

            // Test map: place 2×2 at (1,3) → fits
            // ┌─────────┐
            // │ ■ □ ◄─▲ │ ← widget occupies this area
            // │ □ ■ ◄─▲ │ ← placed from this position
            // └─────────┘
            expect(fitsLeftAbove(testMap, C.pos(1, 3), pb.WidgetSize.LARGE)).toEqual(C.pos(0, 2));
        });

        test('widget does not fit when position would go out of bounds', () => {
            // Empty map: try to place 2×2 at (0,0) → would need row -1 & col -1 (out of bounds)
            //  ↖ would need to extend above & left of this
            // ┌─────────┐
            // │ ✗ □ □ □ │ ← trying to place from here
            // │ □ □ □ □ │
            // └─────────┘
            expect(fitsLeftAbove(emptyMap, C.pos(0, 0), pb.WidgetSize.LARGE)).toBe(null);

            // Empty map: try to place 1×2 at (1,0) → would need col -1 (out of bounds)
            //    ↙ would need to extend left of this
            // ┌─────────┐
            // │ □ □ □ □ │
            // │ ✗ □ □ □ │ ← trying to place from here
            // └─────────┘
            expect(fitsLeftAbove(emptyMap, C.pos(1, 0), pb.WidgetSize.MEDIUM)).toBe(null);
        });

        test('widget does not fit when space is occupied', () => {
            // Test map: try to place 2×2 at (1,1) → would collide with occupied slots
            // ┌─────────┐
            // │ ■ □ □ □ │ ← would collide with this ■
            // │ □ ✗ □ □ │ ← trying to place from here
            // └─────────┘
            expect(fitsLeftAbove(testMap, C.pos(1, 1), pb.WidgetSize.LARGE)).toBe(null);
        });
    });
});

describe('explodeWidgetIntoAtoms', () => {
    describe('explodes widgets into small placeholder atoms', () => {
        test('SMALL widget produces single placeholder', () => {
            const widget: C.Located = { id: 'x', position: C.pos(1, 2), size: pb.WidgetSize.SMALL };
            const result = explodeWidgetIntoAtoms(widget);

            expect(result).toEqual([C.placeholder(1, 2)]);
        });

        test('MEDIUM widget produces 2 horizontal placeholders', () => {
            const widget: C.Located = { id: 'x', position: C.pos(0, 1), size: pb.WidgetSize.MEDIUM };
            const result = explodeWidgetIntoAtoms(widget);

            expect(result).toEqual([C.placeholder(0, 1), C.placeholder(0, 2)]);
        });

        test('LARGE widget produces 4 placeholders in 2x2 grid', () => {
            const widget: C.Located = { id: 'x', position: C.pos(0, 0), size: pb.WidgetSize.LARGE };
            const result = explodeWidgetIntoAtoms(widget);

            expect(result).toEqual([
                C.placeholder(0, 0),
                C.placeholder(0, 1),
                C.placeholder(1, 0),
                C.placeholder(1, 1),
            ]);
        });

        test('FULL widget produces 8 placeholders in 2x4 grid', () => {
            const widget: C.Located = { id: 'x', position: C.pos(0, 0), size: pb.WidgetSize.FULL };
            const result = explodeWidgetIntoAtoms(widget);

            expect(result).toEqual([
                C.placeholder(0, 0),
                C.placeholder(0, 1),
                C.placeholder(0, 2),
                C.placeholder(0, 3),
                C.placeholder(1, 0),
                C.placeholder(1, 1),
                C.placeholder(1, 2),
                C.placeholder(1, 3),
            ]);
        });

        test('works with different positions', () => {
            const widget: C.Located = { id: 'x', position: C.pos(1, 2), size: pb.WidgetSize.LARGE };
            const result = explodeWidgetIntoAtoms(widget);

            expect(result).toEqual([
                C.placeholder(1, 2),
                C.placeholder(1, 3),
                C.placeholder(2, 2),
                C.placeholder(2, 3),
            ]);
        });
    });

    describe('handles edge cases', () => {
        test('throws error for UNSPECIFIED size', () => {
            const widget: C.Located = { id: 'x', position: C.pos(0, 0), size: pb.WidgetSize.UNSPECIFIED };
            expect(() => explodeWidgetIntoAtoms(widget)).toThrow('invalid size');
        });
        test('requires position to be defined', () => {
            const widget = { position: null, size: pb.WidgetSize.SMALL };

            // @ts-expect-error: Intentional error to be caught: C.position is required
            expect(() => explodeWidgetIntoAtoms(widget)).toThrow('position is required');
        });
    });
});

describe('getValidDropSlots', () => {
    describe('calculates valid drop slots for widget movement', () => {
        test('SMALL widget finds all empty slots in empty grid', () => {
            const pool: C.Located[] = [];
            const widget: C.Located = { id: 'test', position: C.pos(0, 0), size: pb.WidgetSize.SMALL };

            const result = getValidDropSlots(pool, widget);

            // All 8 slots should be valid for a SMALL widget in an empty grid
            expect(result).toEqual(
                new Set([
                    C.dropSlotKey(0, 0),
                    C.dropSlotKey(0, 1),
                    C.dropSlotKey(0, 2),
                    C.dropSlotKey(0, 3),
                    C.dropSlotKey(1, 0),
                    C.dropSlotKey(1, 1),
                    C.dropSlotKey(1, 2),
                    C.dropSlotKey(1, 3),
                ]),
            );
        });

        test('MEDIUM widget finds slots with horizontal space', () => {
            const pool: C.Located[] = [
                { id: 'blocker', position: C.pos(0, 0), size: pb.WidgetSize.SMALL }, // Blocks (0,0)
            ];
            const widget: C.Located = { id: 'test', position: C.pos(1, 0), size: pb.WidgetSize.MEDIUM };

            const result = getValidDropSlots(pool, widget);

            // MEDIUM (1×2) widget should find slots where it can fit horizontally
            const expected = new Set([
                C.dropSlotKey(0, 1), // Can fit at cols 1-2
                C.dropSlotKey(0, 2), // Can fit at cols 2-3
                C.dropSlotKey(0, 3), // Can fit at cols 2-3 (right edge placement)
                C.dropSlotKey(1, 0), // Can fit at cols 0-1
                C.dropSlotKey(1, 1), // Can fit at cols 1-2
                C.dropSlotKey(1, 2), // Can fit at cols 2-3
                C.dropSlotKey(1, 3), // Can fit at cols 2-3 (right edge placement)
            ]);
            expect(result).toEqual(expected);
        });

        test('LARGE widget finds slots with 2x2 space', () => {
            const pool: C.Located[] = [
                { id: 'blocker1', position: C.pos(0, 0), size: pb.WidgetSize.SMALL }, // Blocks (0,0)
                { id: 'blocker2', position: C.pos(1, 3), size: pb.WidgetSize.SMALL }, // Blocks (1,3)
            ];
            const widget: C.Located = { id: 'test', position: C.pos(0, 1), size: pb.WidgetSize.LARGE };

            const result = getValidDropSlots(pool, widget);

            // LARGE (2×2) widget should find slots where it can fit as 2x2 grid
            // fitsAround allows multiple placement positions that result in same final placement
            const expected = new Set([
                C.dropSlotKey(0, 1), // Can place at (0,1) -> spans to (0,1)-(0,2)-(1,1)-(1,2)
                C.dropSlotKey(0, 2), // Can place at (0,2) -> fitsAround finds valid placement
                C.dropSlotKey(1, 1), // Can place at (1,1) -> spans to (0,1)-(0,2)-(1,1)-(1,2)
                C.dropSlotKey(1, 2), // Can place at (1,2) -> fitsAround finds valid placement
            ]);
            expect(result).toEqual(expected);
        });

        test('FULL widget finds slots with 2x4 space', () => {
            const pool: C.Located[] = [];
            const widget: C.Located = { id: 'test', position: C.pos(0, 0), size: pb.WidgetSize.FULL };

            const result = getValidDropSlots(pool, widget);

            // FULL (2×4) widget can fit at multiple positions due to fitsAround logic
            const expected = new Set([
                C.dropSlotKey(0, 0), // Top-left placement
                C.dropSlotKey(0, 3), // Top-right placement (fitsAround finds valid placement)
                C.dropSlotKey(1, 0), // Bottom-left placement (fitsAround finds valid placement)
                C.dropSlotKey(1, 3), // Bottom-right placement (fitsAround finds valid placement)
            ]);
            expect(result).toEqual(expected);
        });

        test('excludes the widget being moved from occupancy calculation', () => {
            const widget: C.Located = { id: 'moving', position: C.pos(0, 0), size: pb.WidgetSize.SMALL };
            const pool: C.Located[] = [
                widget, // This widget is being moved, should be excluded
                { id: 'static', position: C.pos(0, 1), size: pb.WidgetSize.SMALL },
            ];

            const result = getValidDropSlots(pool, widget);

            // The moving widget's position should be available, but not the static widget's position
            expect(result.has(C.dropSlotKey(0, 0))).toBe(true); // Original position available
            expect(result.has(C.dropSlotKey(0, 1))).toBe(false); // Blocked by static widget
        });

        test('handles complex layout with mixed widget sizes', () => {
            const pool: C.Located[] = [
                { id: 'large1', position: C.pos(0, 0), size: pb.WidgetSize.LARGE }, // Occupies (0,0)-(0,1)-(1,0)-(1,1)
                { id: 'small1', position: C.pos(0, 3), size: pb.WidgetSize.SMALL }, // Occupies (0,3)
            ];
            const widget: C.Located = { id: 'test', position: C.pos(1, 3), size: pb.WidgetSize.SMALL };

            const result = getValidDropSlots(pool, widget);

            // Available slots for SMALL widget:
            // (0,2) - available
            // (1,2) - available
            // (1,3) - available (widget's original position)
            const expected = new Set([C.dropSlotKey(0, 2), C.dropSlotKey(1, 2), C.dropSlotKey(1, 3)]);
            expect(result).toEqual(expected);
        });

        test('returns empty set when no valid drop slots exist', () => {
            // Fill the entire grid except one slot
            const pool: C.Located[] = [
                { id: '1', position: C.pos(0, 0), size: pb.WidgetSize.SMALL },
                { id: '2', position: C.pos(0, 1), size: pb.WidgetSize.SMALL },
                { id: '3', position: C.pos(0, 2), size: pb.WidgetSize.SMALL },
                { id: '4', position: C.pos(0, 3), size: pb.WidgetSize.SMALL },
                { id: '5', position: C.pos(1, 0), size: pb.WidgetSize.SMALL },
                { id: '6', position: C.pos(1, 1), size: pb.WidgetSize.SMALL },
                { id: '7', position: C.pos(1, 2), size: pb.WidgetSize.SMALL },
                // (1,3) is free
            ];

            // Try to place a LARGE widget - it needs 2x2 space but only 1 slot is free
            const widget: C.Located = { id: 'test', position: C.pos(0, 0), size: pb.WidgetSize.LARGE };

            const result = getValidDropSlots(pool, widget);

            expect(result.size).toBe(0);
        });
    });

    describe('handles edge cases', () => {
        test('requires widget position to be defined', () => {
            const pool: C.Located[] = [];
            const widget = { id: 'test', position: null, size: pb.WidgetSize.SMALL };

            // @ts-expect-error: Intentional error to be caught: position is required
            expect(() => getValidDropSlots(pool, widget)).toThrow('widget.position is required');
        });

        test('requires widget size to be defined', () => {
            const pool: C.Located[] = [];
            const widget = { id: 'test', position: C.pos(0, 0), size: null };

            // @ts-expect-error: Intentional error to be caught: size is required
            expect(() => getValidDropSlots(pool, widget)).toThrow('widget.size is required');
        });

        test('handles widget with same id in pool correctly', () => {
            const widget: C.Located = { id: 'duplicate', position: C.pos(0, 0), size: pb.WidgetSize.SMALL };
            const pool: C.Located[] = [
                { id: 'duplicate', position: C.pos(1, 0), size: pb.WidgetSize.SMALL }, // Same id, different position
                { id: 'other', position: C.pos(0, 1), size: pb.WidgetSize.SMALL },
            ];

            const result = getValidDropSlots(pool, widget);

            // Both positions with id 'duplicate' should be excluded, leaving slots blocked only by 'other'
            expect(result.has(C.dropSlotKey(0, 0))).toBe(true); // Original position available
            expect(result.has(C.dropSlotKey(1, 0))).toBe(true); // Other duplicate position available
            expect(result.has(C.dropSlotKey(0, 1))).toBe(false); // Blocked by 'other'
        });
    });
});

describe('defaultFormifiedValue', () => {
    test('paramString with defaultValue', () => {
        expect(defaultFormifiedValue(paramDef('paramString', 'k', false, { defaultValue: 'hi' }))).toBe('hi');
    });
    test('paramString without defaultValue → empty string', () => {
        expect(defaultFormifiedValue(paramDef('paramString'))).toBe('');
    });
    test('paramInteger with defaultValue', () => {
        expect(defaultFormifiedValue(paramDef('paramInteger', 'k', false, { defaultValue: 5 }))).toBe('5');
    });
    test('paramInteger optional, no default → null', () => {
        expect(defaultFormifiedValue(paramDef('paramInteger', 'k', true))).toBeNull();
    });
    test('paramDouble with defaultValue', () => {
        expect(defaultFormifiedValue(paramDef('paramDouble', 'k', false, { defaultValue: 1.5 }))).toBe('1.5');
    });
    test('paramBoolean with defaultValue true', () => {
        expect(defaultFormifiedValue(paramDef('paramBoolean', 'k', false, { defaultValue: true }))).toBe(true);
    });
    test('paramBoolean without defaultValue → false', () => {
        expect(defaultFormifiedValue(paramDef('paramBoolean'))).toBe(false);
    });
    test('paramTimezone with defaultValue', () => {
        expect(defaultFormifiedValue(paramDef('paramTimezone', 'k', true, { defaultValue: 'Europe/Prague' }))).toBe(
            'Europe/Prague',
        );
    });
    test('paramTimezone optional, no default → null', () => {
        expect(defaultFormifiedValue(paramDef('paramTimezone', 'k', true))).toBeNull();
    });
    test('paramArray → one row per default item, each with its own id', () => {
        const rows = defaultFormifiedValue(countsDef({ defaultValue: [wireInteger(1), wireInteger(2)] }));
        if (!Array.isArray(rows)) throw new Error('expected rows');
        expect(rows.map(row => row.value)).toEqual(['1', '2']);
        expect(new Set(rows.map(row => row.id)).size).toBe(2);
    });
    test('paramArray of objects → a row per default item, an omitted field at its default', () => {
        const rows = defaultFormifiedValue(linksDef({ defaultValue: [wireRow({ label: 'Home' })] }));
        if (!Array.isArray(rows)) throw new Error('expected rows');
        expect(rows.map(row => row.value)).toEqual([{ label: 'Home', url: '' }]);
    });
});

function linksDef(overrides: Record<string, any> = {}): pb.ManifestParamDefinition {
    return paramDef('paramArray', 'links', false, {
        items: {
            kind: {
                case: 'paramObject',
                value: {
                    fields: [
                        { key: 'label', name: 'Label', kind: { case: 'paramString', value: {} } },
                        { key: 'url', name: 'URL', isOptional: true, kind: { case: 'paramString', value: {} } },
                    ],
                },
            },
        },
        maxItems: 3,
        ...overrides,
    });
}

function wireRow(fields: Record<string, string>): pb.FieldValue {
    const wire = Object.entries(fields).map(([key, value]) => [
        key,
        pb.create(pb.FieldValueSchema, { kind: { case: 'stringValue', value } }),
    ]);
    return pb.create(pb.FieldValueSchema, {
        kind: { case: 'objectValue', value: { fields: Object.fromEntries(wire) } },
    });
}

function countsDef(overrides: Record<string, any> = {}): pb.ManifestParamDefinition {
    return paramDef('paramArray', 'counts', false, {
        items: { kind: { case: 'paramInteger', value: { max: 5 } } },
        maxItems: 3,
        ...overrides,
    });
}

const listNamedConstructor = paramDef('paramArray', 'constructor', false, {
    items: { kind: { case: 'paramInteger', value: {} } },
    maxItems: 3,
});

function wireInteger(value: number): pb.FieldValue {
    return pb.create(pb.FieldValueSchema, { kind: { case: 'integerValue', value } });
}

function rows(...values: string[]): ListItem[] {
    return values.map(listItem);
}

describe('parseFormifiedValue', () => {
    test.each([
        ['required', paramDef('paramString'), ''],
        ['not a number', paramDef('paramDouble'), 'abc'],
        ['not an integer', paramDef('paramInteger'), '1.5'],
        ['past int32', paramDef('paramInteger'), '2147483648'],
    ] as const)('an error of %s goes through the catalog', (_case, def, raw) => {
        const r = parseFormifiedValueIn(pseudoIntl, def, raw);
        if (r.ok || !r.error) throw new Error('the value parsed');
        const english = parseFormifiedValue(def, raw);
        if (english.ok) throw new Error('the value parsed');
        expect(r.error).not.toBe(english.error);
    });
    test.each([
        ['paramString', null],
        ['paramString', ''],
        ['paramInteger', ''],
        ['paramTimezone', null],
        ['paramTimezone', ''],
    ] as const)('a required %s left empty (%j) is refused', (kind, raw) => {
        const r = parseFormifiedValue(paramDef(kind), raw);
        if (!r.ok) expect(r.error).toBe('Missing value!');
        else throw new Error('expected error');
    });
    test('paramString optional, empty → nullValue', () => {
        const r = parseFormifiedValue(paramDef('paramString', 'k', true), '');
        if (r.ok) expect(r.value.kind.case).toBe('nullValue');
        else throw new Error('expected ok');
    });
    test('paramString non-empty → stringValue', () => {
        const r = parseFormifiedValue(paramDef('paramString'), 'hi');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'stringValue', value: 'hi' });
        else throw new Error('expected ok');
    });
    test('a value outside its schema still goes, for the device to refuse', () => {
        const def = paramDef('paramString', 'k', false, { maxLength: 1 });
        const r = parseFormifiedValue(def, 'too long');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'stringValue', value: 'too long' });
        else throw new Error('expected ok');
    });

    test('paramInteger optional, empty → nullValue', () => {
        const r = parseFormifiedValue(paramDef('paramInteger', 'k', true), '');
        if (r.ok) expect(r.value.kind.case).toBe('nullValue');
        else throw new Error('expected ok');
    });
    test('paramInteger "42" → integerValue 42', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '42');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'integerValue', value: 42 });
        else throw new Error('expected ok');
    });
    test('paramInteger "1.5" → error (non-integer)', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '1.5');
        expect(r.ok).toBe(false);
    });
    test('paramInteger "abc" → error', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), 'abc');
        expect(r.ok).toBe(false);
    });
    test('paramInteger "  -3  " trimmed → integerValue -3', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '  -3  ');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'integerValue', value: -3 });
        else throw new Error('expected ok');
    });

    test('paramDouble "1.5" → doubleValue 1.5', () => {
        const r = parseFormifiedValue(paramDef('paramDouble'), '1.5');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'doubleValue', value: 1.5 });
        else throw new Error('expected ok');
    });
    test('paramDouble "1e" → error (NaN)', () => {
        const r = parseFormifiedValue(paramDef('paramDouble'), '1e');
        expect(r.ok).toBe(false);
    });

    test('paramInteger past its own bounds still goes, for the device to refuse', () => {
        const r = parseFormifiedValue(paramDef('paramInteger', 'k', false, { max: 10 }), '11');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'integerValue', value: 11 });
        else throw new Error('expected ok');
    });
    test('paramInteger is capped at int32', () => {
        const r = parseFormifiedValue(paramDef('paramInteger', 'k', false, { min: 0 }), '2147483648');
        if (!r.ok) expect(r.error).toBe('Must be at most 2147483647');
        else throw new Error('expected error');
    });
    test('paramInteger is floored at int32', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '-2147483649');
        if (!r.ok) expect(r.error).toBe('Must be at least -2147483648');
        else throw new Error('expected error');
    });
    test('paramInteger "1e10" → error (past int32)', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '1e10');
        if (!r.ok) expect(r.error).toBe('Must be at most 2147483647');
        else throw new Error('expected error');
    });
    test('paramInteger past int32 names its own bound where it has one', () => {
        const high = parseFormifiedValue(paramDef('paramInteger', 'k', false, { max: 100 }), '99999999999');
        if (!high.ok) expect(high.error).toBe('Must be at most 100');
        else throw new Error('expected error');
        const low = parseFormifiedValue(paramDef('paramInteger', 'k', false, { min: -5 }), '-99999999999');
        if (!low.ok) expect(low.error).toBe('Must be at least -5');
        else throw new Error('expected error');
    });
    test('paramInteger at the int32 edges → integerValue', () => {
        for (const n of [-2_147_483_648, 2_147_483_647]) {
            const r = parseFormifiedValue(paramDef('paramInteger'), String(n));
            if (r.ok) expect(r.value.kind).toEqual({ case: 'integerValue', value: n });
            else throw new Error(`expected ${n} to parse`);
        }
    });
    test('paramDouble past int32 → doubleValue', () => {
        const r = parseFormifiedValue(paramDef('paramDouble'), '3000000000');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'doubleValue', value: 3_000_000_000 });
        else throw new Error('expected ok');
    });

    test('paramBoolean true → booleanValue true', () => {
        const r = parseFormifiedValue(paramDef('paramBoolean'), true);
        if (r.ok) expect(r.value.kind).toEqual({ case: 'booleanValue', value: true });
        else throw new Error('expected ok');
    });

    test('paramTimezone optional null → nullValue (system timezone)', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', true), null);
        if (r.ok) expect(r.value.kind.case).toBe('nullValue');
        else throw new Error('expected ok');
    });
    test('paramTimezone optional empty string → nullValue (lenient)', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', true), '');
        if (r.ok) expect(r.value.kind.case).toBe('nullValue');
        else throw new Error('expected ok');
    });
    test('paramTimezone "Europe/Prague" → stringValue', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', true), 'Europe/Prague');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'stringValue', value: 'Europe/Prague' });
        else throw new Error('expected ok');
    });

    test('paramArray within its bounds → listValue of the parsed items', () => {
        const r = parseFormifiedValue(countsDef(), rows('3', '1'));
        if (!r.ok) throw new Error('expected ok');
        if (r.value.kind.case !== 'listValue') throw new Error('expected listValue');
        expect(r.value.kind.value.items.map(item => item.kind)).toEqual([
            { case: 'integerValue', value: 3 },
            { case: 'integerValue', value: 1 },
        ]);
    });
    test('paramArray items without a wire form → errors at their indices, none on the list', () => {
        const r = parseFormifiedValue(countsDef(), rows('1', 'abc', '1.5'));
        if (r.ok) throw new Error('expected error');
        expect(r.error).toBeUndefined();
        expect(r.items).toEqual([undefined, { error: 'Must be a number' }, { error: 'Must be a whole number' }]);
    });
    test('paramArray of objects → a struct per row, a blank optional field as null', () => {
        const r = parseFormifiedValue(linksDef(), [listItem({ label: 'Home', url: '' })]);
        if (!r.ok) throw new Error('expected ok');
        if (r.value.kind.case !== 'listValue') throw new Error('expected listValue');
        const [row] = r.value.kind.value.items;
        if (row?.kind.case !== 'objectValue') throw new Error('expected objectValue');
        expect(row.kind.value.fields.label.kind).toEqual({ case: 'stringValue', value: 'Home' });
        expect(row.kind.value.fields.url.kind.case).toBe('nullValue');
    });
    test('paramArray past its bounds or with repeats still goes, for the device to refuse', () => {
        const r = parseFormifiedValue(
            countsDef({ maxItems: 1, uniqueItems: { case: 'whole', value: {} } }),
            rows('1', '1'),
        );
        expect(r.ok).toBe(true);
    });
});

describe('widgetParamsToFormifiedState', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'w',
        name: 'W',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [
            paramDef('paramString', 'name'),
            paramDef('paramInteger', 'count', true),
            paramDef('paramBoolean', 'enabled'),
            paramDef('paramTimezone', 'tz', true),
        ],
    });

    test('undefined struct → defaults from manifest', () => {
        const r = widgetParamsToFormifiedState(manifest, undefined);
        expect(r.name).toBe('');
        expect(r.count).toBeNull();
        expect(r.enabled).toBe(false);
        expect(r.tz).toBeNull();
    });

    test('integer value from BE → string', () => {
        const struct = pb.create(pb.FieldValuesSchema, {
            fields: {
                count: pb.create(pb.FieldValueSchema, { kind: { case: 'integerValue', value: 7 } }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.count).toBe('7');
    });

    test('null value from BE → null', () => {
        const struct = pb.create(pb.FieldValuesSchema, {
            fields: {
                tz: pb.create(pb.FieldValueSchema, {
                    kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
                }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.tz).toBeNull();
    });

    test('null value from BE for boolean → false', () => {
        const struct = pb.create(pb.FieldValuesSchema, {
            fields: {
                enabled: pb.create(pb.FieldValueSchema, {
                    kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
                }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.enabled).toBe(false);
    });

    test('list value from BE → one row per item', () => {
        const withList = pb.create(pb.WidgetManifestSchema, { params: [countsDef()] });
        const struct = pb.create(pb.FieldValuesSchema, {
            fields: {
                counts: pb.create(pb.FieldValueSchema, {
                    kind: { case: 'listValue', value: { items: [wireInteger(4), wireInteger(2)] } },
                }),
            },
        });
        const r = widgetParamsToFormifiedState(withList, struct);
        if (!Array.isArray(r.counts)) throw new Error('expected rows');
        expect(r.counts.map(row => row.value)).toEqual(['4', '2']);
    });

    test('unknown keys are not surfaced', () => {
        const struct = pb.create(pb.FieldValuesSchema, {
            fields: {
                ghost: pb.create(pb.FieldValueSchema, { kind: { case: 'stringValue', value: 'x' } }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect((r as Record<string, unknown>).ghost).toBeUndefined();
    });

    test('a param named like an Object member, not stored, takes its default', () => {
        const named = pb.create(pb.WidgetManifestSchema, { params: [listNamedConstructor] });
        const r = widgetParamsToFormifiedState(named, pb.create(pb.FieldValuesSchema));
        expect(r).toEqual({ constructor: [] });
    });
});

describe('buildFieldValues', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'w',
        name: 'W',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [
            paramDef('paramString', 'name'),
            paramDef('paramInteger', 'count'),
            paramDef('paramBoolean', 'enabled'),
        ],
    });

    test('all valid → ok with struct', () => {
        const r = buildFieldValues(manifest, { name: 'a', count: '3', enabled: true });
        expect(r.ok).toBe(true);
        if (r.ok) {
            expect(r.value.fields.name.kind).toEqual({ case: 'stringValue', value: 'a' });
            expect(r.value.fields.count.kind).toEqual({ case: 'integerValue', value: 3 });
            expect(r.value.fields.enabled.kind).toEqual({ case: 'booleanValue', value: true });
        }
    });

    test('one bad field → ok=false with error in fields[key]', () => {
        const r = buildFieldValues(manifest, { name: 'a', count: 'abc', enabled: true });
        expect(r.ok).toBe(false);
        if (!r.ok) {
            expect(r.errors.fields.count).toBeTruthy();
            expect(r.errors.fields.name).toBeFalsy();
        }
    });

    test('missing required → error', () => {
        const r = buildFieldValues(manifest, { count: '1', enabled: false });
        expect(r.ok).toBe(false);
        if (!r.ok) expect(r.errors.fields.name).toBeTruthy();
    });

    test('a param named like an Object member, left out, builds from its default', () => {
        const named = pb.create(pb.WidgetManifestSchema, { params: [listNamedConstructor] });
        expect(buildFieldValues(named, {}).ok).toBe(true);
    });

    test('a bad list item → error in items[key][index]', () => {
        const withList = pb.create(pb.WidgetManifestSchema, { params: [countsDef()] });
        const r = buildFieldValues(withList, { counts: rows('1', 'abc') });
        if (r.ok) throw new Error('expected error');
        expect(r.errors.fields.counts).toBeUndefined();
        expect(r.errors.items?.counts).toEqual([undefined, { errors: ['Must be a number'] }]);
    });
});

describe('fitStoredParams', () => {
    const counter = (key: string, defaultValue: number) => paramDef('paramInteger', key, false, { defaultValue });

    test('keeps a stored value as sent, even one the form would refuse', () => {
        const manifest = pb.create(pb.WidgetManifestSchema, { params: [countsDef()] });
        const stored = pb.create(pb.FieldValuesSchema, {
            fields: {
                counts: pb.create(pb.FieldValueSchema, {
                    kind: { case: 'listValue', value: { items: [wireInteger(9)] } },
                }),
            },
        });

        expect(fitStoredParams(manifest, stored)).toEqual(stored);
    });

    test('leaves out a key the manifest no longer declares', () => {
        const manifest = pb.create(pb.WidgetManifestSchema, { params: [counter('count', 1)] });
        const stored = pb.create(pb.FieldValuesSchema, {
            fields: { count: wireInteger(7), retired: wireInteger(3) },
        });

        expect(Object.keys(fitStoredParams(manifest, stored).fields)).toEqual(['count']);
    });

    test('gives a key the manifest added its default', () => {
        const manifest = pb.create(pb.WidgetManifestSchema, { params: [counter('count', 1), counter('added', 4)] });
        const stored = pb.create(pb.FieldValuesSchema, { fields: { count: wireInteger(7) } });

        const fitted = fitStoredParams(manifest, stored);
        expect(fitted.fields.count.kind).toEqual({ case: 'integerValue', value: 7 });
        expect(fitted.fields.added.kind).toEqual({ case: 'integerValue', value: 4 });
    });

    test('defaults an unstored key that names an Object member', () => {
        const key: string = 'toString';
        const manifest = pb.create(pb.WidgetManifestSchema, { params: [counter(key, 4)] });

        const fitted = fitStoredParams(manifest, pb.create(pb.FieldValuesSchema));
        expect(fitted.fields[key]?.kind).toEqual({ case: 'integerValue', value: 4 });
    });
});

describe('params form errors', () => {
    const formErrors = (over: Partial<ParamsFormErrors> = {}): ParamsFormErrors => ({
        global: [],
        fields: {},
        ...over,
    });
    const count = paramDef('paramInteger', 'count', false, { max: 5 });

    describe('hasFieldErrors', () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            params: [paramDef('paramInteger', 'count'), countsDef()],
            credentials: [{ key: 'pool', typeId: 'braiins-pool', label: 'Pool' }],
        });

        test('no errors, or rows that carry none, do not count', () => {
            expect(hasFieldErrors(null, manifest)).toBe(false);
            expect(hasFieldErrors(formErrors(), manifest)).toBe(false);
            const blank = formErrors({ items: { counts: [undefined, { errors: [], fields: { label: [] } }] } });
            expect(hasFieldErrors(blank, manifest)).toBe(false);
        });

        test("a row field's error counts", () => {
            const form = formErrors({ items: { counts: [{ fields: { label: ['Missing value!'] } }] } });
            expect(hasFieldErrors(form, manifest)).toBe(true);
        });

        test('a param the form draws counts, by its field and by its rows', () => {
            expect(hasFieldErrors(formErrors({ fields: { count: ['Must be at most 5'] } }), manifest)).toBe(true);
            const rowError = formErrors({ items: { counts: [{ errors: ['Must be at most 5'] }] } });
            expect(hasFieldErrors(rowError, manifest)).toBe(true);
        });

        test('a credential slot the form draws counts', () => {
            expect(hasFieldErrors(formErrors({ credentials: { pool: ['Unknown account'] } }), manifest)).toBe(true);
        });

        test('what the form does not draw has nowhere to show, so it does not count', () => {
            const dropped = formErrors({
                fields: { retired: ['Unknown setting!'] },
                items: { oldRows: [{ errors: ['Unknown field!'] }] },
                credentials: { oldSlot: ['Unknown account'] },
            });
            expect(hasFieldErrors(dropped, manifest)).toBe(false);
        });
    });

    describe('itemErrorsOf', () => {
        test("keeps each row's and each field's first violation, and the rows without any", () => {
            const form = formErrors({
                items: {
                    links: [
                        undefined,
                        { errors: ['First', 'Second'], fields: { label: ['Missing value!', 'Too long'] } },
                    ],
                },
            });
            expect(itemErrorsOf(form, 'links')).toEqual([
                undefined,
                { error: 'First', fields: { label: 'Missing value!' } },
            ]);
        });

        test('a key without item errors has none', () => {
            expect(itemErrorsOf(null, 'links')).toBeUndefined();
            expect(itemErrorsOf(formErrors(), 'links')).toBeUndefined();
        });

        test('a key named like an Object member finds no inherited errors', () => {
            expect(itemErrorsOf(formErrors({ items: {} }), 'constructor')).toBeUndefined();
        });
    });

    describe('clearFieldError', () => {
        test('nothing to clear stays nothing', () => {
            expect(clearFieldError(null, 'count')).toBeNull();
        });

        test("clears the key's field error and the status, keeping other keys and what no field shows", () => {
            const form = formErrors({
                status: 'Invalid params',
                global: ['Unsupported size'],
                fields: { count: ['Must be at most 5'], name: ['Missing value!'] },
                items: { counts: [{ errors: ['Must be at most 5'] }] },
            });
            const cleared = clearFieldError(form, 'count');
            expect(cleared?.status).toBeUndefined();
            expect(cleared).toEqual({
                global: ['Unsupported size'],
                fields: { name: ['Missing value!'] },
                items: { counts: [{ errors: ['Must be at most 5'] }] },
            });
        });

        test("clears a list key's row errors and the status", () => {
            const form = formErrors({
                status: 'Invalid params',
                items: { counts: [{ errors: ['Must be at most 5'] }] },
            });
            const cleared = clearFieldError(form, 'counts');
            expect(cleared?.status).toBeUndefined();
            expect(cleared).toEqual({ global: [], fields: {}, items: {} });
        });

        test("keeps the bindings' violations, which no param edit addresses", () => {
            const form = formErrors({
                fields: { count: ['Must be at most 5'] },
                credentials: { pool: ['Account not found'] },
            });
            expect(clearFieldError(form, 'count')?.credentials).toEqual({ pool: ['Account not found'] });
        });

        test('keeps the status when the key had no error', () => {
            const form = formErrors({ status: 'Invalid params', fields: { name: ['Missing value!'] } });
            expect(clearFieldError(form, 'count')?.status).toBe('Invalid params');
        });

        test('a key named like an Object member had no error to clear', () => {
            const form = formErrors({ status: 'Invalid params', items: {} });
            expect(clearFieldError(form, 'constructor')?.status).toBe('Invalid params');
        });
    });

    describe('revalidateField', () => {
        test('a value that now parses clears the key', () => {
            const form = formErrors({ status: 'Invalid params', fields: { count: ['Must be at most 5'] } });
            expect(revalidateField(form, count, '3')).toEqual({ global: [], fields: {}, items: {} });
        });

        test('an invalid scalar sets its field error, keeping the other keys', () => {
            const form = formErrors({ status: 'Invalid params', fields: { name: ['Missing value!'] } });
            expect(revalidateField(form, count, 'abc')).toEqual({
                status: 'Invalid params',
                global: [],
                fields: { name: ['Missing value!'], count: ['Must be a number'] },
                items: {},
            });
        });

        test("an invalid list sets its rows' errors", () => {
            expect(revalidateField(null, countsDef(), rows('1', 'abc'))).toEqual({
                global: [],
                fields: {},
                items: { counts: [undefined, { errors: ['Must be a number'] }] },
            });
        });
    });

    describe("what the form's notice and toasts show", () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            params: [paramDef('paramInteger', 'count')],
            credentials: [{ key: 'pool', typeId: 'braiins-pool', label: 'Pool' }],
        });

        test('takes in the violations on keys the manifest no longer declares', () => {
            const form = formErrors({
                global: ['Unsupported size'],
                fields: { count: ['Must be at most 5'], retired: ['Unknown setting!'] },
                items: { gone: [undefined, { errors: ['Not a row'], fields: { label: ['Missing value!'] } }] },
                credentials: { pool: ['Account not found'], old: ['Unknown slot'] },
            });
            expect(unplacedErrors(form, manifest)).toEqual([
                'Unsupported size',
                'Unknown setting!',
                'Not a row',
                'Missing value!',
                'Unknown slot',
            ]);
        });

        test('leaves out the status while a field shows the reason', () => {
            const form = formErrors({
                status: 'Invalid params',
                global: ['Unsupported size'],
                fields: { count: ['Must be at most 5'] },
            });
            expect(formNotices(form, manifest)).toEqual(['Unsupported size']);
        });

        test('shows the status when no field explains it', () => {
            const form = formErrors({ status: 'Invalid params', global: ['Unsupported size'] });
            expect(formNotices(form, manifest)).toEqual(['Invalid params', 'Unsupported size']);
        });

        test("the form's own check keeps the device's violations it cannot judge", () => {
            const local = formErrors({ fields: { count: ['Must be a number'] } });
            const previous = formErrors({
                status: 'Invalid params',
                global: ['Unsupported size'],
                fields: { count: ['Must be at most 5'] },
                credentials: { pool: ['Account not found'] },
            });
            const merged = withLocalErrors(local, previous);
            expect(merged.status).toBeUndefined();
            expect(merged).toEqual({
                global: ['Unsupported size'],
                fields: { count: ['Must be a number'] },
                items: {},
                credentials: { pool: ['Account not found'] },
            });
        });

        test("a row's local fault keeps the device's word on its list", () => {
            const local = formErrors({
                fields: { counts: undefined },
                items: { counts: [{ errors: ['Must be a number'] }] },
            });
            const previous = formErrors({ fields: { counts: ['Must have at most 2 items'] } });
            expect(withLocalErrors(local, previous).fields.counts).toEqual(['Must have at most 2 items']);
        });

        test("the form's own check keeps the device's word on keys the manifest no longer declares", () => {
            const local = formErrors({ fields: { count: ['Must be a number'] } });
            const previous = formErrors({ fields: { retired: ['Unknown setting!'] } });
            expect(unplacedErrors(withLocalErrors(local, previous), manifest)).toEqual(['Unknown setting!']);
        });

        test('a new size clears the violations on the request and keeps the fields', () => {
            const form = formErrors({
                status: 'Invalid params',
                global: ['Unsupported size'],
                fields: { count: ['Must be a number'] },
            });
            const cleared = clearGlobalErrors(form);
            expect(cleared?.status).toBeUndefined();
            expect(cleared).toEqual({ global: [], fields: { count: ['Must be a number'] } });
        });

        test("rebinding a slot clears that slot's violation and the status", () => {
            const form = formErrors({
                status: 'Invalid params',
                credentials: { pool: ['Account not found'], api: ['Account not found'] },
            });
            const cleared = clearSlotError(form, 'pool');
            expect(cleared?.status).toBeUndefined();
            expect(cleared?.credentials).toEqual({ pool: [], api: ['Account not found'] });
        });

        test('rebinding a slot without a violation keeps the rest as it was', () => {
            const form = formErrors({ status: 'Invalid params', fields: { count: ['Must be at most 5'] } });
            expect(clearSlotError(form, 'pool')).toBe(form);
        });

        test('a refused save names its status even while a field shows the reason', () => {
            const form = formErrors({
                status: 'Invalid params',
                global: ['Unsupported size'],
                fields: { count: ['Must be at most 5'] },
            });
            expect(saveNotices(form, manifest)).toEqual(['Invalid params', 'Unsupported size']);
        });
    });
});

describe('credential bindings', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        credentials: [{ key: 'pool', typeId: 'braiins-pool', label: 'Pool' }],
    });
    const accounts = [
        pb.create(pb.AccountSchema, { id: 'pool-a', typeId: 'braiins-pool' }),
        pb.create(pb.AccountSchema, { id: 'token-1', typeId: 'generic-token' }),
    ];

    test('a set of fitting accounts is valid', () => {
        expect(credentialBindingsValid(manifest, accounts, { pool: 'pool-a' })).toBe(true);
    });

    test('an account of the wrong type is not', () => {
        expect(credentialBindingsValid(manifest, accounts, { pool: 'token-1' })).toBe(false);
    });

    test('a slot the manifest no longer declares is not', () => {
        expect(credentialBindingsValid(manifest, accounts, { retired: 'pool-a' })).toBe(false);
    });

    test('bindings to a deleted account are left out', () => {
        expect(withoutDeletedAccounts({ pool: 'pool-a', backup: 'gone' }, accounts)).toEqual({ pool: 'pool-a' });
    });

    test('untouched bindings stay out of every write', () => {
        const session = { manifest, accounts, original: { pool: 'pool-a' } };
        for (const write of ['preview', 'cancel', 'done'] as const) {
            expect(credentialBindingsFor(write, session)).toBeUndefined();
        }
    });

    test('a preview carries the edited set and its cancel the original', () => {
        const session = { manifest, accounts, original: { pool: 'pool-a' }, edited: {} };
        expect(credentialBindingsFor('preview', session)).toEqual({ bindings: {} });
        expect(credentialBindingsFor('cancel', session)).toEqual({ bindings: { pool: 'pool-a' } });
    });

    test('a preview and its cancel leave bindings out while the original set could not be sent back', () => {
        const session = { manifest, accounts, original: { pool: 'token-1' }, edited: { pool: 'pool-a' } };
        expect(credentialBindingsFor('preview', session)).toBeUndefined();
        expect(credentialBindingsFor('cancel', session)).toBeUndefined();
    });

    test('done carries the edited set whatever the original was', () => {
        const session = { manifest, accounts, original: { pool: 'token-1' }, edited: { pool: 'pool-a' } };
        expect(credentialBindingsFor('done', session)).toEqual({ bindings: { pool: 'pool-a' } });
    });

    test('a slot the manifest no longer declares is left out of every write', () => {
        const session = {
            manifest,
            accounts,
            original: { pool: 'pool-a', retired: 'pool-a' },
            edited: { pool: '', retired: 'pool-a' },
        };
        expect(credentialBindingsFor('preview', session)).toEqual({ bindings: { pool: '' } });
        expect(credentialBindingsFor('cancel', session)).toEqual({ bindings: { pool: 'pool-a' } });
        expect(credentialBindingsFor('done', session)).toEqual({ bindings: { pool: '' } });
    });
});

describe('explainTimeout', () => {
    test('a timeout gets a message the page can show as it is', () => {
        const explained = explainTimeout(fakeIntlProp, new ConnectError('deadline exceeded', Code.DeadlineExceeded));
        expect(ConnectError.from(explained).rawMessage).toBe("The device didn't answer in time.");
    });

    test('any other error passes through unchanged', () => {
        const refused = new ConnectError('refused', Code.FailedPrecondition);
        expect(explainTimeout(fakeIntlProp, refused)).toBe(refused);
    });
});
