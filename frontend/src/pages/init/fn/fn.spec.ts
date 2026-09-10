// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { describe, test, expect } from '@rstest/core';

import { Code, ConnectError } from '@connectrpc/connect';
import { URLS } from '@/constants';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import { awaitSetupOutcome, isMiningSetup, postSetupDestination } from './fn';

const miner = deckCapabilities({ miningSupported: true, ethernetSupported: true, productName: 'Braiins Mini Miner' });
const deck = deckCapabilities();
const http = { protocol: 'http:' };

describe('isMiningSetup', () => {
    test('true only when the device reports mining support', () => {
        expect(isMiningSetup(miner)).toBe(true);
        expect(isMiningSetup(deck)).toBe(false);
    });
});

describe('postSetupDestination', () => {
    test('a display device logs in within the app', () => {
        expect(postSetupDestination(deck, { protocol: 'dhcp' }, http)).toEqual({ route: URLS.auth.login });
    });

    test('a miner on DHCP leaves for the site root on the same host', () => {
        expect(postSetupDestination(miner, { protocol: 'dhcp' }, http)).toEqual({ url: '/' });
    });

    test('a miner given a static address is followed there', () => {
        expect(postSetupDestination(miner, { protocol: 'static', staticAddress: '192.168.1.126' }, http)).toEqual({
            url: 'http://192.168.1.126/',
        });
    });

    test('a static protocol without an address falls back to the same host', () => {
        expect(postSetupDestination(miner, { protocol: 'static', staticAddress: '' }, http)).toEqual({ url: '/' });
    });
});

describe('awaitSetupOutcome', () => {
    const fast = { attempts: 3, delayMs: 0 };

    test('a device still answering the setup data means the request was lost', async () => {
        await expect(awaitSetupOutcome(async () => ({}), fast)).resolves.toBe('pending');
    });

    test('a device refusing the setup data has left setup', async () => {
        const probe = async () => {
            throw new ConnectError('not in setup', Code.FailedPrecondition);
        };
        await expect(awaitSetupOutcome(probe, fast)).resolves.toBe('applied');
    });

    test('a device coming back after a transport failure is judged by its answer', async () => {
        const answers = [new ConnectError('down', Code.Unavailable), new ConnectError('gone', Code.FailedPrecondition)];
        const probe = async () => {
            throw answers.shift();
        };
        await expect(awaitSetupOutcome(probe, fast)).resolves.toBe('applied');
    });

    test('a device that never answers is unreachable', async () => {
        const probe = async () => {
            throw new ConnectError('down', Code.Unavailable);
        };
        await expect(awaitSetupOutcome(probe, fast)).resolves.toBe('unreachable');
    });
});
