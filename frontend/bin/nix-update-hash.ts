#!/usr/bin/env -S pnpm exec tsx
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
 * Update the pnpm deps FOD (Fixed-Output Derivation) hash.
 *
 * Usage:
 *   ./bin/nix-update-hash.ts           # Fetch the pnpm deps and pin their hash
 */

import { fileURLToPath } from 'node:url';
import { $, cd, spinner, path, fs, echo, chalk } from 'zx';

const PATH_SELF = fileURLToPath(import.meta.url);
const PATH_BIN = path.dirname(PATH_SELF);
const PATH_FRONTEND = path.dirname(PATH_BIN);
const PATH_ROOT = path.dirname(PATH_FRONTEND);

cd(PATH_ROOT);
const PATH_NODE_MODULES_NIX = path.resolve(PATH_FRONTEND, 'nix/node-modules.nix');

const timeStart: DOMHighResTimeStamp = performance.now();
const res = await spinner('', () => {
    const args = ['-L', '--log-format', 'bar-with-logs', '--no-link', '.#frontendNodeModules.pnpmDeps'];

    // language=bash
    return $({ cwd: PATH_ROOT, nothrow: true, verbose: true, env: process.env })`nix build ${args}`;
});
const timeEnd: DOMHighResTimeStamp = performance.now();
echo(`> Took ${((timeEnd - timeStart) / 1_000).toFixed(2)}s`);

if (res.ok) {
    echo(chalk.green('Hash is up to date, nothing to do!'));
} else {
    const out: string = res.stdall;
    const hashSpecified = out.match(/specified: (.*)\n/)?.[1];
    const hashReceived = out.match(/got:\s+(.*)\n/)?.[1];

    if (!hashSpecified || !hashReceived) {
        echo(chalk.red('The fetch failed without a hash mismatch; see the log above'));
        process.exit(1);
    }

    echo('');
    echo('> specified: ', chalk.yellowBright(hashSpecified));
    echo('>       got: ', chalk.redBright(hashReceived));
    echo('> ');

    const current = fs.readFileSync(PATH_NODE_MODULES_NIX, 'utf-8');
    if (!current.includes(hashSpecified)) {
        echo(chalk.red(`${path.basename(PATH_NODE_MODULES_NIX)} does not contain the specified hash`));
        process.exit(1);
    }
    fs.writeFileSync(PATH_NODE_MODULES_NIX, current.replace(hashSpecified, hashReceived), 'utf-8');

    const underlinedFileName: string = chalk.underline(path.basename(PATH_NODE_MODULES_NIX));
    const message: string = chalk.greenBright(`${underlinedFileName} has been updated`);
    echo(`> ${message}`);
    echo('');
}
