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

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test } from '@rstest/core';

// The components
import * as carbon from '@carbon/react';
import * as components from './index';

interface BiomeConfig {
    linter: {
        rules: {
            style: {
                noRestrictedImports: { options: { paths: Record<string, { importNames: string[] }> } };
            };
        };
    };
}

test('biome restricts every Carbon import that `@/components` has a component of its own for', () => {
    const ours: Record<string, unknown> = components;
    const theirs: Record<string, unknown> = carbon;
    const shadowed = Object.keys(ours).filter(name => name in theirs && ours[name] !== theirs[name]);

    const biome: BiomeConfig = JSON.parse(readFileSync(resolve(process.cwd(), 'biome.json'), 'utf8'));
    const restricted = biome.linter.rules.style.noRestrictedImports.options.paths['@carbon/react'].importNames;

    expect([...restricted].sort()).toEqual(shadowed.sort());
});
