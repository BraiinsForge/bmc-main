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

import { Code, ConnectError } from '@connectrpc/connect';

import * as pb from '@/proto';
import { URLS } from '@/constants';
import { delay } from '@/lib/async';
import type { NetworkProtocol } from '../components/Setup';

/** A miner runs the pool + network form; every other device the localization one. */
export function isMiningSetup(capabilities: null | pb.HardwareCapabilities): boolean {
    return capabilities?.miningSupported === true;
}

export type PostSetupDestination = { route: string } | { url: string };

/**
 * Where the browser goes once device setup succeeds: a display device logs in
 * within this app; a miner leaves for its main page at the site root, at the
 * static address it was just given when there is one.
 */
export function postSetupDestination(
    capabilities: null | pb.HardwareCapabilities,
    network: { protocol?: NetworkProtocol; staticAddress?: string },
    location: { protocol: string },
): PostSetupDestination {
    if (!isMiningSetup(capabilities)) return { route: URLS.auth.login };
    if (network.protocol === 'static' && network.staticAddress) {
        return { url: `${location.protocol}//${network.staticAddress}/` };
    }
    return { url: '/' };
}

export type SetupOutcome = 'applied' | 'pending' | 'unreachable';

export const SETUP_PROBE_ATTEMPTS = 15;
export const SETUP_PROBE_DELAY_MS = 2_000;

/**
 * Decides what a transport failure of `SetupDevice` meant by asking for the
 * setup data, which only answers while setup is still pending: a
 * `FailedPrecondition` says the settings were applied, an answer says the
 * request never landed, and a device that stays silent is reported as such.
 */
export async function awaitSetupOutcome(
    probe: () => Promise<unknown>,
    options: { attempts?: number; delayMs?: number; signal?: AbortSignal } = {},
): Promise<SetupOutcome> {
    const { attempts = SETUP_PROBE_ATTEMPTS, delayMs = SETUP_PROBE_DELAY_MS, signal } = options;
    for (let attempt = 0; attempt < attempts; attempt++) {
        if (attempt > 0) await delay(delayMs);
        if (signal?.aborted) throw new ConnectError('setup probe aborted', Code.Canceled);
        try {
            await probe();
            return 'pending';
        } catch (error) {
            if (pb.abort.is(error)) throw error;
            if (ConnectError.from(error).code === Code.FailedPrecondition) return 'applied';
        }
    }
    return 'unreachable';
}
