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

import { afterEach, describe, test, expect, rstest } from '@rstest/core';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { MemoryRouter } from 'react-router';
import { IntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { ConnectError, Code } from '@connectrpc/connect';

import Settings from './Settings';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import { Toaster } from '@/lib/toast';
import AppContext, { getAppContextDefault } from '@/context';
import type { ServiceMocks } from '@/lib/proto';

// Committing an upgrade goes through the confirm-modal context; answer it per test.
// A spy so tests can assert how many times the confirmation was actually asked.
let confirmAnswer = true;
const confirmSpy = rstest.fn(() => Promise.resolve(confirmAnswer));
const appContext = { ...getAppContextDefault(), confirm: confirmSpy };

// Let the pending confirm→run branch run to completion. Needed for assert-a-negative
// checks: a macrotask boundary drains the post-confirm microtasks, so a buggy path
// that ignored a declined answer would already have called `startUpgrade` by the check.
const flushTasks = (): Promise<void> => new Promise(resolve => setTimeout(resolve));

// Fake-timer helper: advance and flush the RPC/debounce promise chain inside act().
const advance = async (ms: number): Promise<void> => {
    await act(async () => {
        await rstest.advanceTimersByTimeAsync(ms);
    });
};

// The server instance id the reconcile polls: the first read is the pre-upgrade
// baseline, later reads return `polledInstanceId` — set it different to simulate the
// app cycling after `finished`.
let baselineInstanceId = 'instance-pre';
let polledInstanceId = 'instance-pre';
let instanceReads = 0;

// jsdom won't let us spy on location.reload directly; swap the whole location.
const REAL_LOCATION = window.location;
function stubReload(): ReturnType<typeof rstest.fn> {
    const reload = rstest.fn();
    Object.defineProperty(window, 'location', {
        configurable: true,
        value: Object.assign(new URL(window.location.href), { reload }),
    });
    return reload;
}

// `mocks.service` wants every method typed; at runtime it only registers what we
// pass. This lets us register a typed subset.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

// A package-only offer: no firmware, so the backend would arbitrate this as an
// AppRestart. All the fold cares about is that it yields an upgradeId to start.
function packageOffer(): pb.CheckForUpgradeResponse {
    return pb.create(pb.CheckForUpgradeResponseSchema, {
        upgradeId: 'upgrade-0',
        disruption: pb.UpgradeDisruption.APP_RESTART,
        packages: pb.create(pb.PackageUpgradePlanSchema, {
            bmcVersion: '26.07',
            changes: [
                pb.create(pb.PackageChangeSchema, {
                    name: 'core',
                    versionFrom: '26.06',
                    versionTo: '26.07',
                    category: 'system',
                }),
            ],
        }),
    });
}

// A StartUpgrade progress event as the client reads it. The mock interceptor
// returns the object verbatim and the fold only consumes `event.case`/`value`.
type Progress = pb.UpgradeProgress;
const pkgPhase = (value: pb.PackageUpgradePhase): Progress => ({ event: { case: 'packagePhase', value } }) as Progress;
const finished: Progress = { event: { case: 'finished', value: {} } } as Progress;

type StartUpgradeMock = ServiceMocks<typeof pb.services.UpgradeService>['startUpgrade'];

function installMocks(startUpgrade: StartUpgradeMock): void {
    mocks.clear();
    registerMocks(pb.services.UpgradeService, {
        checkForUpgrade: () => packageOffer(),
        startUpgrade,
    });
    // Reconcile reads the server instance id: first read = baseline, later reads =
    // `polledInstanceId` (change it to simulate a restart).
    registerMocks(pb.services.MetadataService, {
        getServerInstance: () => {
            instanceReads += 1;
            return pb.create(pb.GetServerInstanceResponseSchema, {
                serverInstanceId: instanceReads === 1 ? baselineInstanceId : polledInstanceId,
            });
        },
    });
}

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <AppContext.Provider value={appContext}>
                    <MemoryRouter initialEntries={['/#updates']}>
                        <Settings />
                        <Toaster />
                    </MemoryRouter>
                </AppContext.Provider>
            </IntlProvider>
        </HelmetProvider>,
    );
}

const startButton = (): HTMLElement => screen.getByRole('button', { name: /Download & Upgrade/i });

// The offer renders only after the debounced mount (150ms) plus the check RPC;
// on a loaded CI runner that can exceed waitFor's 1s default, so give the wait more room.
const awaitOffer = () => waitFor(() => expect(startButton()).toBeTruthy(), { timeout: 5_000 });

afterEach(() => {
    cleanup();
    mocks.clear();
    confirmAnswer = true;
    confirmSpy.mockClear();
    rstest.useRealTimers();
    baselineInstanceId = 'instance-pre';
    polledInstanceId = 'instance-pre';
    instanceReads = 0;
    Object.defineProperty(window, 'location', { configurable: true, value: REAL_LOCATION });
});

describe('Settings upgrade stream terminal classification (BDK-559)', () => {
    // `ACTIVATING` is emitted before the fallible activation,
    // so a stream error after it must surface as a failure
    // — not be masked as a reboot.
    test('a stream error after ACTIVATING surfaces as a failure, not "restarting"', async () => {
        installMocks(() =>
            (async function* () {
                yield pkgPhase(pb.PackageUpgradePhase.REALIZING);
                yield pkgPhase(pb.PackageUpgradePhase.VERIFYING);
                yield pkgPhase(pb.PackageUpgradePhase.BUILDING);
                yield pkgPhase(pb.PackageUpgradePhase.ACTIVATING);
                throw new ConnectError('mock: activation failed', Code.Internal);
            })(),
        );

        renderPage();
        await awaitOffer();

        fireEvent.click(startButton());

        await waitFor(() => {
            // Never entered the restart/reload overlay...
            expect(screen.queryByText(/restarting/i)).toBeNull();
            // ...the failure surfaced...
            expect(screen.getByText(/mock: activation failed/)).toBeTruthy();
            // ...and the offer is interactive again, not stuck behind a blocking overlay.
            expect(startButton()).toBeTruthy();
        });
    });

    // A package `finished` with no server cycle settles in place:
    // the instance id never changes, so reconcile runs its window
    // out and toasts, never showing the overlay.
    test('a package "finished" with no restart settles with a toast', async () => {
        rstest.useFakeTimers();
        installMocks(() =>
            (async function* () {
                yield pkgPhase(pb.PackageUpgradePhase.REALIZING);
                yield pkgPhase(pb.PackageUpgradePhase.VERIFYING);
                yield pkgPhase(pb.PackageUpgradePhase.BUILDING);
                yield pkgPhase(pb.PackageUpgradePhase.ACTIVATING);
                yield finished;
            })(),
        );

        renderPage();
        await advance(300); // debounced mount (150ms) + initial fetches
        fireEvent.click(startButton());

        // After `finished` the stepper shows the animated "finishing up" row
        // while the reconcile runs — never the restart overlay.
        //
        // The stepper renders twice: inline facts + the blocking overlay, hence getAllByText.
        await advance(2_000);
        expect(screen.getAllByText('Finishing up…').length).toBeGreaterThan(0);
        expect(screen.queryByText(/restarting/i)).toBeNull();

        // A stable instance id settles to the toast once the window elapses.
        await advance(11_000);
        expect(screen.getByText('Packages updated')).toBeTruthy();
        expect(screen.queryByText(/restarting/i)).toBeNull();
    });

    // A package `finished` followed by a server cycle: the instance id changes,
    // so the reconcile reloads the app instead of settling.
    test('a package "finished" that cycles the server reloads', async () => {
        rstest.useFakeTimers();
        polledInstanceId = 'instance-post'; // later reads differ from the baseline
        const reload = stubReload();
        installMocks(() =>
            (async function* () {
                yield pkgPhase(pb.PackageUpgradePhase.ACTIVATING);
                yield finished;
            })(),
        );

        renderPage();
        await advance(300);
        fireEvent.click(startButton());

        // The first poll after `finished` sees the changed id and reloads — no toast.
        await advance(2_000);

        expect(reload).toHaveBeenCalledTimes(1);
        expect(screen.queryByText('Packages updated')).toBeNull();
    });

    // Declining the pre-commit confirmation must not start the run.
    // Asserted on the RPC spy after the confirmation has resolved
    // — the UI-only checks are all true from the initial state,
    // so they would pass before a buggy "ignore the decline"
    // path ever reached `startUpgrade`.
    test('a declined confirmation does not start the upgrade', async () => {
        confirmAnswer = false;
        const startUpgrade = rstest.fn(() =>
            (async function* () {
                yield finished;
            })(),
        );
        installMocks(startUpgrade);

        renderPage();
        await awaitOffer();

        fireEvent.click(startButton());

        // The confirmation is asked and answered "no"; wait for that branch to settle so
        // a path that ignored the answer would already have launched the run.
        await waitFor(() => expect(confirmSpy).toHaveBeenCalledTimes(1));
        await flushTasks();

        expect(startUpgrade).not.toHaveBeenCalled();
        // ...and nothing surfaced: no progress overlay, no toast, offer still interactive.
        expect(screen.queryByText('Upgrading the system')).toBeNull();
        expect(screen.queryByText('Packages updated')).toBeNull();
        expect(startButton()).toBeTruthy();
    });

    // The re-entry guard drops a rapid second click, so a double click asks the
    // confirmation and launches the run exactly once — not twice, where the second run
    // would abort the first via `replace()`.
    test('a double click confirms and starts the upgrade only once', async () => {
        const startUpgrade = rstest.fn(() =>
            (async function* () {
                yield finished;
            })(),
        );
        installMocks(startUpgrade);

        renderPage();
        await awaitOffer();

        fireEvent.click(startButton());
        fireEvent.click(startButton());

        await waitFor(() => expect(startUpgrade).toHaveBeenCalledTimes(1));
        expect(confirmSpy).toHaveBeenCalledTimes(1);
    });
});
