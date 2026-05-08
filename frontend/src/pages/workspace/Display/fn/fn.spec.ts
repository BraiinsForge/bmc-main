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
    defaultParamValue,
} from './fn';

const emptyParams = pb.create(pb.WidgetDataStructSchema, { fields: {} });

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

describe('defaultParamValue', () => {
    test('returns the manifest default for paramString', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'name',
            name: 'Name',
            isOptional: false,
            kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, { defaultValue: 'hi' }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('stringValue');
        if (v.kind.case === 'stringValue') expect(v.kind.value).toBe('hi');
    });

    test('returns empty string for paramString with no default', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'name',
            name: 'Name',
            isOptional: false,
            kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('stringValue');
        if (v.kind.case === 'stringValue') expect(v.kind.value).toBe('');
    });

    test('returns null for an optional paramInteger with no default', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'count',
            name: 'Count',
            isOptional: true,
            kind: { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('nullValue');
    });

    test('returns the manifest default for paramInteger', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'count',
            name: 'Count',
            isOptional: false,
            kind: { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, { defaultValue: 42 }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('integerValue');
        if (v.kind.case === 'integerValue') expect(v.kind.value).toBe(42);
    });

    test('returns false for paramBoolean with no default', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'flag',
            name: 'Flag',
            isOptional: false,
            kind: { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('booleanValue');
        if (v.kind.case === 'booleanValue') expect(v.kind.value).toBe(false);
    });

    test('returns null for unset kind', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'x',
            name: 'X',
            isOptional: true,
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('nullValue');
    });
});
