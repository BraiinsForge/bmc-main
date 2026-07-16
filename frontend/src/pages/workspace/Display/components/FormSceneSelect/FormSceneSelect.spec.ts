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
import { groupByCategory, visibleSections } from './FormSceneSelect';

function manifest(name: string, category: pb.WidgetCategory): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, { uid: name, name, category });
}

const C = pb.WidgetCategory;

describe('groupByCategory', () => {
    test('orders sections deterministically with misc last and skips empty ones', () => {
        const sections = groupByCategory([
            manifest('Weather', C.WEATHER),
            manifest('Other thing', C.MISC),
            manifest('Mining Info', C.MINING),
        ]);
        // No CLOCK/SPACE widgets, so those sections are absent; MISC stays last.
        expect(sections.map(s => s.category)).toEqual([C.MINING, C.WEATHER, C.MISC]);
    });

    test('sorts widgets by name within a section', () => {
        const [section] = groupByCategory([
            manifest('Zulu', C.MINING),
            manifest('Alpha', C.MINING),
            manifest('Mike', C.MINING),
        ]);
        expect(section.widgets.map(w => w.name)).toEqual(['Alpha', 'Mike', 'Zulu']);
    });

    test('buckets unset / unspecified category under misc', () => {
        const sections = groupByCategory([manifest('No category', C.UNSPECIFIED), manifest('Clocky', C.CLOCK)]);
        expect(sections.map(s => s.category)).toEqual([C.CLOCK, C.MISC]);
        const misc = sections.find(s => s.category === C.MISC);
        expect(misc?.widgets.map(w => w.name)).toEqual(['No category']);
    });

    // Guards the order list the way `assertUnreachable` in `categoryLabel` guards
    // the labels: a proto category absent from CATEGORY_ORDER would silently land
    // in MISC instead of its own section, so adding one here fails the build.
    test('every category gets its own section, none silently bucketed into misc', () => {
        const realCategories = Object.values(C).filter(
            (v): v is pb.WidgetCategory => typeof v === 'number' && v !== C.UNSPECIFIED && v !== C.MISC,
        );
        for (const category of realCategories) {
            const [section] = groupByCategory([manifest('w', category)]);
            expect(section?.category).toBe(category);
        }
    });
});

describe('visibleSections', () => {
    const sections = groupByCategory([
        manifest('Mining Info', C.MINING),
        manifest('Clock', C.CLOCK),
        manifest('Weather', C.WEATHER),
    ]);

    test('empty selection shows every section', () => {
        expect(visibleSections(sections, new Set())).toEqual(sections);
    });

    test('a selection keeps only the chosen categories', () => {
        const visible = visibleSections(sections, new Set([C.MINING, C.WEATHER]));
        expect(visible.map(s => s.category)).toEqual([C.MINING, C.WEATHER]);
    });
});
