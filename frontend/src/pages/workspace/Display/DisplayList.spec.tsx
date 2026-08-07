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

import { afterEach, beforeEach, describe, test, expect, rstest } from '@rstest/core';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react/pure';
import { MemoryRouter } from 'react-router';
import { IntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import DisplayList from './DisplayList';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';

// `mocks.service` wants every method typed; at runtime it only registers what we
// pass. This lets us register a typed subset.
type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

// One Clone button per row, id `bmc-display-comp-scene-overview-row-<sceneId>-clone`.
// Used to count rows and read back each row's scene id.
const ROW_ID_PREFIX = 'bmc-display-comp-scene-overview-row-';
const ROW_ID_SUFFIX = '-clone';

function rowSceneIds(container: HTMLElement): string[] {
    const sel = `[id^="${ROW_ID_PREFIX}"][id$="${ROW_ID_SUFFIX}"]`;
    return [...container.querySelectorAll<HTMLElement>(sel)].map(el =>
        el.id.slice(ROW_ID_PREFIX.length, -ROW_ID_SUFFIX.length),
    );
}

function makeScene(id: string): pb.Scene {
    return pb.create(pb.SceneSchema, { id, enabled: true, cycleDurationSec: 30 });
}

// Backend scene store stand-in; the cloneScene mock inserts one fresh scene next
// to the source, like the real backend.
let server: pb.Scene[] = [];
let cloneSeq = 0;

function installMocks(): void {
    mocks.clear();

    registerMocks(pb.services.SceneManagementService, {
        getScenes: () => ({ scenes: server.map(s => ({ ...s })) }),
        cloneScene: ({ req }) => {
            const srcId = req.value;
            const idx = server.findIndex(s => s.id === srcId);
            cloneSeq += 1;
            const base = idx >= 0 ? server[idx] : server[0];
            const copy = makeScene(`${srcId}_c${cloneSeq}`);
            copy.enabled = base.enabled;
            server.splice((idx >= 0 ? idx : server.length - 1) + 1, 0, copy);
            return { value: copy.id };
        },
        getSceneCycling: () => ({}),
        getAvailableWidgets: () => ({ widgets: [] }),
    });

    registerMocks(pb.services.HardwareService, {
        getHardwareCapabilities: () => ({ combinedScenesSupported: false }),
    });

    registerMocks(pb.services.SystemService, {
        getTimezoneList: () => ({ timezones: [] }),
    });
}

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter>
                    <DisplayList />
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );
}

// Advance fake timers and flush the RPC/debounce promise chain inside act().
async function flush(ms = 10): Promise<void> {
    await act(async () => {
        await rstest.advanceTimersByTimeAsync(ms);
    });
}

function clickFirstClone(container: HTMLElement): void {
    const btn = container.querySelector<HTMLElement>(`[id^="${ROW_ID_PREFIX}"][id$="${ROW_ID_SUFFIX}"]`);
    if (!btn) throw new Error('no clone button rendered');
    fireEvent.click(btn);
}

beforeEach(() => {
    rstest.useFakeTimers();
    cloneSeq = 0;
    server = [makeScene('A')];
    installMocks();
});

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.useRealTimers();
});

describe('screen cycling transition effect', () => {
    const CYCLE_MENU_ID = 'bmc-display-list-cycle-form-menu';
    const EFFECT_DROPDOWN_ID = 'bmc-display-list-cycle-transition-effect';

    let saved: pb.SceneCycling[];

    beforeEach(() => {
        saved = [];
        registerMocks(pb.services.SceneManagementService, {
            getSceneCycling: () => ({
                sceneCycling: pb.create(pb.SceneCyclingSchema, {
                    automaticCyclingEnabled: true,
                    automaticCyclingDefaultDurationSec: 30,
                    transition: pb.SceneCyclingTransition.SLIDE,
                }),
            }),
            setSceneCycling: ({ req }) => {
                if (req.sceneCycling) saved.push(req.sceneCycling);
                return {};
            },
        });
    });

    // Open the screen-cycling overflow menu and return the effect dropdown's
    // toggle button (Carbon puts the given id on the ListBox wrapper).
    async function openCyclingForm(): Promise<HTMLElement> {
        const trigger = document.getElementById(CYCLE_MENU_ID);
        if (!trigger) throw new Error('cycling menu trigger not rendered');
        fireEvent.click(trigger);
        await flush(50);
        const toggle = document.querySelector<HTMLElement>(`#${EFFECT_DROPDOWN_ID} button`);
        if (!toggle) throw new Error('transition effect dropdown not rendered');
        return toggle;
    }

    function effectOptions(): HTMLElement[] {
        return [...document.querySelectorAll<HTMLElement>('[role="option"]')];
    }

    test('the selector shows the effect loaded from the backend', async () => {
        renderPage();
        await flush();

        const toggle = await openCyclingForm();

        expect(toggle.textContent).toContain('Slide');
    });

    test('the selector offers Slide, Fade and None in that order', async () => {
        renderPage();
        await flush();

        const toggle = await openCyclingForm();
        fireEvent.click(toggle);

        expect(effectOptions().map(el => el.textContent?.trim())).toEqual(['Slide', 'Fade', 'None']);
    });

    test('choosing None submits it with the other cycling settings intact', async () => {
        renderPage();
        await flush();

        const toggle = await openCyclingForm();
        fireEvent.click(toggle);
        const none = effectOptions().find(el => el.textContent?.includes('None'));
        if (!none) throw new Error('None option not rendered');
        fireEvent.click(none);
        await flush();

        expect(saved).toEqual([
            expect.objectContaining({
                transition: pb.SceneCyclingTransition.NONE,
                automaticCyclingEnabled: true,
                automaticCyclingDefaultDurationSec: 30,
            }),
        ]);
    });
});

describe('DisplayList scene clone (BDK-527)', () => {
    test('cloning a scene never renders two rows for the same scene id', async () => {
        const { container } = renderPage();
        await flush();

        expect(rowSceneIds(container)).toEqual(['A']);

        clickFirstClone(container);

        // Optimistic update applied: must not render two rows with the same id.
        const ids = rowSceneIds(container);
        expect(new Set(ids).size).toBe(ids.length);
    });

    test('a clone settles to the backend scene list with a unique id', async () => {
        // Real timers: the debounced reload that swaps the placeholder for the real
        // scene is a genuine ~1s wait, awkward to model with fake timers here.
        rstest.useRealTimers();

        const { container } = renderPage();
        await waitFor(() => expect(rowSceneIds(container)).toEqual(['A']));

        clickFirstClone(container);

        // After the reload, rows mirror the backend exactly — original + the one
        // real clone, distinct ids, no leftover placeholder or duplicate.
        await waitFor(() => expect(rowSceneIds(container)).toEqual(server.map(s => s.id)), { timeout: 2_000 });
        const ids = rowSceneIds(container);
        expect(ids.length).toBe(2);
        expect(new Set(ids).size).toBe(ids.length);
    });

    test('while a clone settles, the whole list locks', async () => {
        const { container } = renderPage();
        await flush();
        expect(rowSceneIds(container)).toEqual(['A']);

        clickFirstClone(container);

        const optimisticId = rowSceneIds(container).find(id => id.startsWith('__bmc-optimistic-clone__'));
        expect(optimisticId).toBeTruthy();

        const cloneBtn = (sceneId: string) =>
            container.querySelector<HTMLButtonElement>(`[id="${ROW_ID_PREFIX}${sceneId}${ROW_ID_SUFFIX}"]`);

        // Both the placeholder and the existing source row are locked until reload.
        expect(cloneBtn(optimisticId as string)?.disabled).toBe(true);
        expect(cloneBtn('A')?.disabled).toBe(true);
    });
});
