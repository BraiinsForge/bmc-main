#!/usr/bin/env -S yarn tsx

import { fileURLToPath } from 'node:url';
import { $, spinner, path, fs, echo, chalk } from 'zx';

const PATH_SELF = fileURLToPath(import.meta.url);
const PATH_BIN = path.dirname(PATH_SELF);
const PATH_FRONTEND = path.dirname(PATH_BIN);
const PATH_ROOT = path.dirname(PATH_FRONTEND);

const PATH_YARN_FILES = path.resolve(PATH_FRONTEND, 'nix/yarn-files.nix');

const timeStart: number = performance.now();
const res = await spinner('', () => {
    // language=bash
    return $({ cwd: PATH_ROOT, nothrow: true, verbose: true })`nix build -L '.#frontend'`;
});
const timeEnd: number = performance.now();
echo(`> Took ${timeEnd - timeStart / 1000}s`);

if (res.ok) echo(chalk.green('Build passed, nothing to do!'));
else {
    const out: string = res.stdall;
    const hashSpecified = out.match(/specified: (.*)\n/)?.[1];
    const hashReceived = out.match(/got:\s+(.*)\n/)?.[1];

    const patchedYarnFiles = fs.readFileSync(PATH_YARN_FILES, 'utf-8').replace(hashSpecified, hashReceived);
    fs.writeFileSync(PATH_YARN_FILES, patchedYarnFiles, 'utf-8');

    echo('');
    echo('> specified: ', chalk.yellowBright(hashSpecified));
    echo('>       gpt: ', chalk.redBright(hashReceived));
    echo('> ');

    const underlinedFileName: string = chalk.underline(path.basename(PATH_YARN_FILES));
    const message: string = chalk.greenBright(`${underlinedFileName} has been updated`);
    echo(`> ${message}`);
    echo('');
}
