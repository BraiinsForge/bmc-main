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

import { echo, chalk, question } from 'zx';

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type MaybeArray<T> = T | T[];

export interface Step {
    title: string;
    info?: string[];
    validate(): boolean | Promise<boolean>;
    command?(): MaybeArray<string>;
    run?(): void | Promise<void>;
    // If validation fails, should we abort? (default: false = can skip/continue)
    required?: boolean;
    // Message to show when validation fails (for required steps)
    failureHelp?: string[];
}

// ─────────────────────────────────────────────────────────────────────────────
// Step runner
// ─────────────────────────────────────────────────────────────────────────────

export async function promptYNS(msg: string): Promise<'y' | 'n' | 's'> {
    const answer = await question(`${msg} [y/n/s] `);
    const c = answer.toLowerCase()[0];
    if (c === 'y') return 'y';
    if (c === 's') return 's';
    return 'n';
}

export async function runStep(index: number, step: Step): Promise<boolean> {
    echo(chalk.bold(`\n[${index + 1}] ${step.title}`));

    if (step.info) for (const line of step.info) echo(`    ${line}`);

    const valid = await step.validate();

    if (valid) {
        echo(chalk.green('    ✓ Already configured'));
        return true;
    }

    if (step.required) {
        echo(chalk.red('    ✗ Validation failed'));
        if (step.failureHelp) {
            echo('');
            for (const line of step.failureHelp) echo(`    ${line}`);
        }
        if (step.command) {
            echo('');
            echo('    Manual command:');
            const cmds = step.command();
            for (const cmd of [cmds].flat()) echo(chalk.cyan(`    ${cmd}`));
        }
        echo('');
        const cont = await promptYNS('    Continue anyway?');
        return cont !== 'n';
    }

    echo(chalk.yellow('    ○ Not configured'));

    if (step.command) {
        echo('');
        const cmds = step.command();
        for (const cmd of [cmds].flat()) echo(chalk.cyan(`    ${cmd}`));
        echo('');
    }

    if (!step.run) {
        echo('    (no automatic action available)');
        return true;
    }

    const action = await promptYNS('    Apply?');
    if (action === 'y') {
        await step.run();
        echo(chalk.green('    ✓ Done'));
    } else if (action === 's') {
        echo('    Skipped.');
    } else {
        echo('    Run the command above manually.');
    }

    return true;
}

export async function runSteps(steps: Step[]): Promise<boolean> {
    for (let i = 0; i < steps.length; i++) {
        const ok = await runStep(i, steps[i]);
        if (!ok) return false;
    }
    return true;
}
