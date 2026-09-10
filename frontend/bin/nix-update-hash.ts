#!/usr/bin/env -S yarn tsx
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

/**
 * Update yarn-files FOD (Fixed-Output Derivation) hash.
 *
 * Usage:
 *   ./bin/nix-update-hash.ts           # Build for current platform
 */

import { fileURLToPath } from 'node:url';
import { $, cd, spinner, path, fs, echo, chalk } from 'zx';

const PATH_SELF = fileURLToPath(import.meta.url);
const PATH_BIN = path.dirname(PATH_SELF);
const PATH_FRONTEND = path.dirname(PATH_BIN);
const PATH_ROOT = path.dirname(PATH_FRONTEND);

cd(PATH_ROOT);
const PATH_YARN_FILES = path.resolve(PATH_FRONTEND, 'nix/yarn-files.nix');

const timeStart: DOMHighResTimeStamp = performance.now();
const res = await spinner('', () => {
    const args = ['-L', '--log-format', 'bar-with-logs', '.#frontend'];

    // language=bash
    return $({ cwd: PATH_ROOT, nothrow: true, verbose: true, env: process.env })`nix build ${args}`;
});
const timeEnd: DOMHighResTimeStamp = performance.now();
echo(`> Took ${((timeEnd - timeStart) / 1_000).toFixed(2)}s`);

// language=bash
await $`rm -rf ./result`;

if (res.ok) {
    echo(chalk.green('Build passed, nothing to do!'));
} else {
    const out: string = res.stdall;
    const hashSpecified = out.match(/specified: (.*)\n/)?.[1];
    const hashReceived = out.match(/got:\s+(.*)\n/)?.[1];

    echo('');
    echo('> specified: ', chalk.yellowBright(hashSpecified));
    echo('>       got: ', chalk.redBright(hashReceived));
    echo('> ');

    if (!hashSpecified || !hashReceived) {
        echo(chalk.red('Failed to parse hash from build output'));
        process.exit(1);
    }

    const patchedYarnFiles = fs
        .readFileSync(PATH_YARN_FILES, 'utf-8')
        // It can be present multiple times since we do platform specific hashes
        .replaceAll(hashSpecified, hashReceived);
    fs.writeFileSync(PATH_YARN_FILES, patchedYarnFiles, 'utf-8');

    const underlinedFileName: string = chalk.underline(path.basename(PATH_YARN_FILES));
    const message: string = chalk.greenBright(`${underlinedFileName} has been updated`);
    echo(`> ${message}`);
    echo('');
}
