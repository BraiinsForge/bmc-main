#!/usr/bin/env -S yarn tsx

/**
 * Update yarn-files FOD (Fixed-Output Derivation) hash.
 *
 * Usage:
 *   ./bin/nix-update-hash.ts           # Build for current platform
 *   ./bin/nix-update-hash.ts --darwin  # Build for x86_64-darwin via remote builder
 *
 * The --darwin flag builds on a remote macOS machine (ci-macos.dev.ii.zone).
 * This requires SSH access to the builder - run ./bin/setup-darwin-builder.ts first.
 * Uses --store to build directly on remote, bypassing nix-daemon to use your SSH agent.
 */

import { fileURLToPath } from 'node:url';
import { $, cd, spinner, path, fs, echo, chalk, argv } from 'zx';

const PATH_SELF = fileURLToPath(import.meta.url);
const PATH_BIN = path.dirname(PATH_SELF);
const PATH_FRONTEND = path.dirname(PATH_BIN);
const PATH_ROOT = path.dirname(PATH_FRONTEND);

cd(PATH_ROOT);
const PATH_YARN_FILES = path.resolve(PATH_FRONTEND, 'nix/yarn-files.nix');

/** @see https://gitlab.ii.zone/nix/infra/-/blob/master/modules/darwin.nix */
const DARWIN_STORE = 'ssh-ng://badmin@ci-macos.dev.ii.zone';

const useDarwin = argv.darwin || argv.d;

const timeStart: DOMHighResTimeStamp = performance.now();
const res = await spinner('', () => {
    const args = ['-L', '--log-format', 'bar-with-logs', '.#frontend'];

    if (useDarwin) {
        // Build on remote darwin machine, evaluate locally
        args.push('--store', DARWIN_STORE);
        args.push('--eval-store', 'auto');
        args.push('--system', 'x86_64-darwin');
    }

    // When using darwin builder, bypass nix-daemon to use user's SSH agent
    const env = useDarwin ? { ...process.env, NIX_REMOTE: '' } : process.env;

    // language=bash
    return $({ cwd: PATH_ROOT, nothrow: true, verbose: true, env })`nix build ${args}`;
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
