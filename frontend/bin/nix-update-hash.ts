#!/usr/bin/env -S yarn tsx

import { fileURLToPath } from 'node:url';
import { $, spinner, path, fs, echo, chalk } from 'zx';

const PATH_SELF = fileURLToPath(import.meta.url);
const PATH_BIN = path.dirname(PATH_SELF);
const PATH_FRONTEND = path.dirname(PATH_BIN);
const PATH_ROOT = path.dirname(PATH_FRONTEND);

const PATH_YARN_FILES = path.resolve(PATH_FRONTEND, 'nix/yarn-files.nix');

const timeStart: DOMHighResTimeStamp = performance.now();
const res = await spinner('', () => {
    // language=bash
    return $({ cwd: PATH_ROOT, nothrow: true, verbose: true })`nix build -L '.#frontend'`;
});
const timeEnd: DOMHighResTimeStamp = performance.now();
echo(`> Took ${(timeEnd - timeStart).toFixed(2)}ms`);

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
