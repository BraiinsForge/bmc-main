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
    defaultFormifiedValue,
    widgetParamsToFormifiedState,
    parseFormifiedValue,
    buildWidgetDataStruct,
} from './fn';

function paramDef(
    kindCase: pb.ManifestParamDefinition['kind']['case'],
    key = 'k',
    isOptional = false,
    overrides: Record<string, any> = {},
): pb.ManifestParamDefinition {
    let kind: pb.ManifestParamDefinition['kind'];
    switch (kindCase) {
        case 'paramString':
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema, overrides) };
            break;
        case 'paramInteger':
            kind = { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, overrides) };
            break;
        case 'paramDouble':
            kind = { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema, overrides) };
            break;
        case 'paramBoolean':
            kind = { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, overrides) };
            break;
        case 'paramTimezone':
            kind = { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema, overrides) };
            break;
        default:
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema) };
    }
    return pb.create(pb.ManifestParamDefinitionSchema, { key, name: 'K', isOptional, kind });
}

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
            kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, { defaultValue: 'hi' }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'stringValue', value: 'hi' });
    });

    test('returns empty string for paramString with no default', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'name',
            name: 'Name',
            kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'stringValue', value: '' });
    });

    test('returns nullValue for paramInteger with no defaultValue', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'count',
            name: 'Count',
            kind: { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('nullValue');
    });

    test('returns the manifest default for paramInteger', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'count',
            name: 'Count',
            kind: { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, { defaultValue: 42 }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'integerValue', value: 42 });
    });

    test('falls back to false for paramBoolean with no default', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'flag',
            name: 'Flag',
            kind: { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'booleanValue', value: false });
    });

    test('returns the manifest default for paramDouble', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'ratio',
            name: 'Ratio',
            kind: { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema, { defaultValue: 3.14 }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'doubleValue', value: 3.14 });
    });

    test('returns nullValue for paramDouble with no defaultValue', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'ratio',
            name: 'Ratio',
            kind: { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('nullValue');
    });

    test('returns the manifest default for paramBoolean with defaultValue true', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'flag',
            name: 'Flag',
            kind: { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, { defaultValue: true }) },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'booleanValue', value: true });
    });

    test('returns the manifest default for paramTimezone', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'tz',
            name: 'Timezone',
            kind: {
                case: 'paramTimezone',
                value: pb.create(pb.ParamTimezoneSchema, { defaultValue: 'Europe/Prague' }),
            },
        });
        const v = defaultParamValue(def);
        expect(v.kind).toMatchObject({ case: 'stringValue', value: 'Europe/Prague' });
    });

    test('returns nullValue for paramTimezone with no defaultValue', () => {
        const def = pb.create(pb.ManifestParamDefinitionSchema, {
            key: 'tz',
            name: 'Timezone',
            kind: { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema, {}) },
        });
        const v = defaultParamValue(def);
        expect(v.kind.case).toBe('nullValue');
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
});

describe('parseFormifiedValue', () => {
    test('paramString required, null → error', () => {
        const r = parseFormifiedValue(paramDef('paramString'), null);
        expect(r.ok).toBe(false);
    });
    test('paramString required, empty → error', () => {
        const r = parseFormifiedValue(paramDef('paramString'), '');
        expect(r.ok).toBe(false);
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

    test('paramInteger required, empty → error', () => {
        const r = parseFormifiedValue(paramDef('paramInteger'), '');
        expect(r.ok).toBe(false);
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

    test('paramInteger below min → error', () => {
        const r = parseFormifiedValue(paramDef('paramInteger', 'k', false, { min: 5 }), '3');
        if (!r.ok) expect(r.error).toBe('Must be at least 5');
        else throw new Error('expected error');
    });
    test('paramInteger above max → error', () => {
        const r = parseFormifiedValue(paramDef('paramInteger', 'k', false, { max: 10 }), '11');
        if (!r.ok) expect(r.error).toBe('Must be at most 10');
        else throw new Error('expected error');
    });
    test('paramDouble below min → error', () => {
        const r = parseFormifiedValue(paramDef('paramDouble', 'k', false, { min: 0.5 }), '0.1');
        if (!r.ok) expect(r.error).toBe('Must be at least 0.5');
        else throw new Error('expected error');
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
    test('paramTimezone required null → Required error', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', false), null);
        if (!r.ok) expect(r.error).toBe('Value is required');
        else throw new Error('expected error');
    });
    test('paramTimezone required empty string → Required error', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', false), '');
        if (!r.ok) expect(r.error).toBe('Value is required');
        else throw new Error('expected error');
    });
    test('paramTimezone "Europe/Prague" → stringValue', () => {
        const r = parseFormifiedValue(paramDef('paramTimezone', 'k', true), 'Europe/Prague');
        if (r.ok) expect(r.value.kind).toEqual({ case: 'stringValue', value: 'Europe/Prague' });
        else throw new Error('expected ok');
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
        const struct = pb.create(pb.WidgetDataStructSchema, {
            fields: {
                count: pb.create(pb.WidgetDataValueSchema, { kind: { case: 'integerValue', value: 7 } }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.count).toBe('7');
    });

    test('null value from BE → null', () => {
        const struct = pb.create(pb.WidgetDataStructSchema, {
            fields: {
                tz: pb.create(pb.WidgetDataValueSchema, {
                    kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
                }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.tz).toBeNull();
    });

    test('null value from BE for boolean → false', () => {
        const struct = pb.create(pb.WidgetDataStructSchema, {
            fields: {
                enabled: pb.create(pb.WidgetDataValueSchema, {
                    kind: { case: 'nullValue', value: pb.create(pb.EmptySchema) },
                }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect(r.enabled).toBe(false);
    });

    test('unknown keys are not surfaced', () => {
        const struct = pb.create(pb.WidgetDataStructSchema, {
            fields: {
                ghost: pb.create(pb.WidgetDataValueSchema, { kind: { case: 'stringValue', value: 'x' } }),
            },
        });
        const r = widgetParamsToFormifiedState(manifest, struct);
        expect((r as Record<string, unknown>).ghost).toBeUndefined();
    });
});

describe('buildWidgetDataStruct', () => {
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
        const r = buildWidgetDataStruct(manifest, { name: 'a', count: '3', enabled: true });
        expect(r.ok).toBe(true);
        if (r.ok) {
            expect(r.value.fields.name.kind).toEqual({ case: 'stringValue', value: 'a' });
            expect(r.value.fields.count.kind).toEqual({ case: 'integerValue', value: 3 });
            expect(r.value.fields.enabled.kind).toEqual({ case: 'booleanValue', value: true });
        }
    });

    test('one bad field → ok=false with error in fields[key]', () => {
        const r = buildWidgetDataStruct(manifest, { name: 'a', count: 'abc', enabled: true });
        expect(r.ok).toBe(false);
        if (!r.ok) {
            expect(r.errors.fields.count).toBeTruthy();
            expect(r.errors.fields.name).toBeFalsy();
        }
    });

    test('missing required → error', () => {
        const r = buildWidgetDataStruct(manifest, { count: '1', enabled: false });
        expect(r.ok).toBe(false);
        if (!r.ok) expect(r.errors.fields.name).toBeTruthy();
    });
});
