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
