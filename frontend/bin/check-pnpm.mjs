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

/**
 * Fail unless the pnpm on PATH has the major `devEngines.packageManager` declares.
 *
 * Plain node rather than a `pnpm exec` script: it has to run before the pnpm it checks.
 *
 * Usage:
 *   node ./bin/check-pnpm.mjs
 */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const { devEngines } = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf-8'));
const declared = devEngines.packageManager.version;

// pnpm 11+ refuses even `--version` on a mismatched major, so its own check is off for this one call.
const env = { ...process.env, pnpm_config_pm_on_fail: 'ignore' };
const running = execFileSync('pnpm', ['--version'], { encoding: 'utf-8', env }).trim();

if (running.split('.')[0] !== declared.split('.')[0]) {
    console.error(`pnpm ${running} is on PATH, but package.json declares pnpm ${declared}`);
    process.exit(1);
}
