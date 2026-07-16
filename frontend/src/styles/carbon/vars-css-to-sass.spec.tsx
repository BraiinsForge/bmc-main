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

import { describe, test, expect } from '@rstest/core';

import fs from 'node:fs';
import path from 'node:path';

describe('CDS css variables mapping to sass', () => {
    const filePath = path.join(__dirname, 'vars-css-to-sass.scss');
    const fileString = fs.readFileSync(filePath, 'utf8');

    const varLines: string[] = fileString
        .split('\n')
        // Get the variable lines
        .filter(line => line.trim().startsWith('$'));

    /**
     * Make sure there are no typos in the variable names and values.
     * The variable name should be the same as the value.
     * @example: $color-primary: v(color-primary);
     */
    describe('should have the same name and value', () => {
        test.each<[string]>(varLines.map(x => [x]))('%p', line => {
            // Split the variable line into name and value
            const match = line.match(/\$([\w-]+): v\('(.+)'\);/);
            if (!match) throw new Error(`Invalid variable line: ${line}`);

            const [_, name, value] = match;
            expect(name).toEqual(value);
        });
    });
});
