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
 * Setup script for configuring the macOS nix remote builder.
 * This enables building darwin-specific FOD hashes (like yarn-files) from a Linux host.
 */

import { $, echo, os, fs, path, chalk } from 'zx';
import { type Step, runSteps } from './_.ts';

$.verbose = false;

/** @see https://gitlab.ii.zone/nix/infra/-/blob/master/modules/darwin.nix */
const DARWIN_BUILDER = {
    host: 'ci-macos.dev.ii.zone',
    user: 'badmin',
    system: 'x86_64-darwin',
    maxJobs: 100,
    speedFactor: 1,
    features: ['nixos-test', 'benchmark', 'big-parallel'],
    publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKctuCBoyBO8onRYhjBCiCvIAS6vd+HaNpVZHH3yQ5lZ',
};

const pathHome = os.homedir();
const pathSshDir = path.join(pathHome, '.ssh');
const pathKnownHosts = path.join(pathSshDir, 'known_hosts');
const pathNixConfUser = path.join(pathHome, '.config', 'nix', 'nix.conf');
const pathNixConfSystem = '/etc/nix/nix.conf';

// ─────────────────────────────────────────────────────────────────────────────
// Step definitions
// ─────────────────────────────────────────────────────────────────────────────

const steps: Step[] = [
    {
        title: 'SSH Known Hosts',
        info: [
            `Host: ${chalk.underline(DARWIN_BUILDER.host)}`,
            '',
            "The builder's SSH host key must be in your known_hosts to prevent",
            'MITM attacks and allow non-interactive connections.',
        ],
        validate() {
            if (!fs.existsSync(pathKnownHosts)) return false;
            const content = fs.readFileSync(pathKnownHosts, 'utf-8');
            return content.includes(DARWIN_BUILDER.host) && content.includes(DARWIN_BUILDER.publicKey);
        },
        command() {
            const entry = `${DARWIN_BUILDER.host} ${DARWIN_BUILDER.publicKey}`;
            return `echo '${entry}' >> ${pathKnownHosts}`;
        },
        run() {
            const entry = `${DARWIN_BUILDER.host} ${DARWIN_BUILDER.publicKey}`;
            if (!fs.existsSync(pathSshDir)) fs.mkdirSync(pathSshDir, { mode: 0o700, recursive: true });
            fs.appendFileSync(pathKnownHosts, `${entry}\n`);
        },
    },
    {
        title: 'SSH Access',
        info: [`Testing connection to ${DARWIN_BUILDER.user}@${DARWIN_BUILDER.host}...`],
        async validate() {
            try {
                const result = await $({
                    nothrow: true,
                    timeout: '10s',
                })`ssh -o ConnectTimeout=5 -o BatchMode=yes ${DARWIN_BUILDER.user}@${DARWIN_BUILDER.host} echo ok`;
                return result.exitCode === 0 && result.stdout.trim() === 'ok';
            } catch {
                return false;
            }
        },
        command() {
            return `ssh ${DARWIN_BUILDER.user}@${DARWIN_BUILDER.host} echo ok`;
        },
        required: true,
        failureHelp: [
            'Your SSH key must be authorized on the darwin builder.',
            'Employee keys should already be authorized.',
            '',
            'Troubleshooting:',
            '- Ensure you have an SSH key: ssh-keygen -t ed25519',
            '- Verify your key is in ssh-agent: ssh-add -l',
            '- Check with infra team if your key should be authorized',
        ],
    },
    {
        title: 'Nix Configuration',
        info: ['Nix needs to know about the remote builder to offload darwin builds.'],
        validate() {
            const pathConf = fs.existsSync(pathNixConfUser) ? pathNixConfUser : pathNixConfSystem;
            if (!fs.existsSync(pathConf)) return false;
            return fs.readFileSync(pathConf, 'utf-8').includes(DARWIN_BUILDER.host);
        },
        command() {
            const pathSshKey =
                [path.join(pathHome, '.ssh', 'id_ed25519'), path.join(pathHome, '.ssh', 'id_rsa')].find(
                    fs.existsSync,
                ) ?? path.join(pathHome, '.ssh', 'id_ed25519');

            const builderLine = [
                `ssh-ng://${DARWIN_BUILDER.user}@${DARWIN_BUILDER.host}`,
                DARWIN_BUILDER.system,
                pathSshKey,
                DARWIN_BUILDER.maxJobs,
                DARWIN_BUILDER.speedFactor,
                DARWIN_BUILDER.features.join(','),
            ].join(' ');

            return [`builders = ${builderLine}`, 'builders-use-substitutes = true'];
        },
        run() {
            const pathConf = fs.existsSync(pathNixConfUser) ? pathNixConfUser : pathNixConfUser;
            const pathConfDir = path.dirname(pathConf);

            const pathSshKey =
                [path.join(pathHome, '.ssh', 'id_ed25519'), path.join(pathHome, '.ssh', 'id_rsa')].find(
                    fs.existsSync,
                ) ?? path.join(pathHome, '.ssh', 'id_ed25519');

            const builderLine = [
                `ssh-ng://${DARWIN_BUILDER.user}@${DARWIN_BUILDER.host}`,
                DARWIN_BUILDER.system,
                pathSshKey,
                DARWIN_BUILDER.maxJobs,
                DARWIN_BUILDER.speedFactor,
                DARWIN_BUILDER.features.join(','),
            ].join(' ');

            const content = [
                '',
                '# macOS remote builder for darwin-specific builds',
                `builders = ${builderLine}`,
                'builders-use-substitutes = true',
                '',
            ].join('\n');

            if (!fs.existsSync(pathConfDir)) fs.mkdirSync(pathConfDir, { recursive: true });
            if (fs.existsSync(pathConf)) fs.appendFileSync(pathConf, content);
            else fs.writeFileSync(pathConf, `${content.trim()}\n`);
        },
    },
];

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

async function main() {
    echo(chalk.bold('═'.repeat(60)));
    echo(chalk.bold('  macOS Nix Remote Builder Setup'));
    echo(chalk.bold('═'.repeat(60)));
    echo('');
    echo(`This script configures your machine to use ${chalk.underline(DARWIN_BUILDER.host)}`);
    echo('for building darwin-specific derivations (e.g., yarn-files FOD hashes).');

    const ok = await runSteps(steps);
    if (!ok) {
        echo(chalk.red('\nSetup aborted.'));
        process.exit(1);
    }

    // Usage info
    echo(chalk.bold(`\n[${steps.length + 1}] Usage`));
    echo('');
    echo('    Build yarn-files for darwin:');
    echo(chalk.cyan('    nix build .#frontend --system x86_64-darwin'));
    echo('');
    echo('    Or use the update script (updates hash automatically):');
    echo(chalk.cyan('    ./bin/nix-update-hash.ts --darwin'));
    echo('');
    echo('    Note: First build may be slow as nix copies dependencies to the builder.');

    echo('');
    echo(chalk.bold('═'.repeat(60)));
    echo(chalk.green('  Setup complete!'));
    echo(chalk.bold('═'.repeat(60)));
}

main().catch(() => process.exit(1));
