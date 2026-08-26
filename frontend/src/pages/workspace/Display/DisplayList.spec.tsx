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
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { Code, ConnectError } from '@connectrpc/connect';
import { MemoryRouter } from 'react-router';
import { IntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';

import DisplayList from './DisplayList';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { Toaster } from '@/lib/toast';

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
const LIMIT_ERROR = 'running widget limit exceeded: 56 running, operation would activate 1, maximum 56';
const LIMIT_MESSAGE = 'Running widget limit reached.';

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
        getScenes: () => ({
            scenes: server.map(s => ({ ...s })),
            runningWidgetCount: 1,
            maxRunningWidgetCount: 56,
        }),
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
        updateWidget: () => ({}),
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
                    <Toaster />
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

    test('a rejected clone removes its optimistic row immediately', async () => {
        registerMocks(pb.services.SceneManagementService, {
            cloneScene: () => {
                throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
            },
        });
        const { container } = renderPage();
        await flush();

        clickFirstClone(container);
        await flush();

        expect(rowSceneIds(container)).toEqual(['A']);
        expect(document.body.textContent).toContain(LIMIT_MESSAGE);
    });
});

describe('running widget limit', () => {
    test('disabling a scene updates the toggle before the request settles', async () => {
        let resolveUpdate: () => void = () => undefined;
        registerMocks(pb.services.SceneManagementService, {
            updateScene: () =>
                new Promise(resolve => {
                    resolveUpdate = () => resolve({});
                }),
        });
        renderPage();
        await flush();

        const toggle = document.getElementById(`${ROW_ID_PREFIX}A-enabled`);
        if (!toggle) throw new Error('scene toggle not rendered');
        expect(toggle.getAttribute('aria-checked')).toBe('true');

        fireEvent.click(toggle);

        expect(toggle.getAttribute('aria-checked')).toBe('false');
        resolveUpdate();
        await flush();
    });

    test('a rejected enable leaves the scene disabled and explains the running widget limit', async () => {
        server[0].enabled = false;
        registerMocks(pb.services.SceneManagementService, {
            updateScene: () => {
                throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
            },
        });
        renderPage();
        await flush();

        const toggle = document.getElementById(`${ROW_ID_PREFIX}A-enabled`);
        if (!toggle) throw new Error('scene toggle not rendered');
        expect(toggle.getAttribute('aria-checked')).toBe('false');

        fireEvent.click(toggle);
        await flush();

        expect(toggle.getAttribute('aria-checked')).toBe('false');
        expect(document.body.textContent).toContain(LIMIT_MESSAGE);
    });

    test('a rejected fullscreen add does not open the widget editor and explains the running widget limit', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            addFullscreenScene: () => {
                throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
            },
        });
        renderPage();
        await flush();

        fireEvent.click(screen.getByRole('button', { name: 'Add New' }));
        fireEvent.click(screen.getByRole('menuitem', { name: 'Full Screen' }));
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Clock' }));
        await flush();

        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
        expect(document.body.textContent).toContain(LIMIT_MESSAGE);
    });

    test('a rejected new fullscreen preview removes the created scene', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        const created = pb.create(pb.SceneSchema, {
            id: 'B',
            enabled: false,
            kind: {
                case: 'fullscreen',
                value: pb.create(pb.Scene_FullscreenSchema, {
                    widget: pb.create(pb.WidgetSchema, {
                        id: 'widget-b',
                        config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                    }),
                }),
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            addFullscreenScene: () => {
                server.push(created);
                return { value: created.id };
            },
            getScene: () => ({ scene: created, runningWidgetCount: 56, maxRunningWidgetCount: 56 }),
            removeScene: ({ req }) => {
                server = server.filter(scene => scene.id !== req.value);
                return {};
            },
            previewScene: () =>
                (async function* () {
                    yield* [];
                    throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
                })(),
        });
        renderPage();
        await flush();

        fireEvent.click(screen.getByRole('button', { name: 'Add New' }));
        fireEvent.click(screen.getByRole('menuitem', { name: 'Full Screen' }));
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Clock' }));
        await flush();
        await flush();

        expect(server.some(scene => scene.id === created.id)).toBe(false);
        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
        expect(document.body.textContent).toContain(LIMIT_MESSAGE);
    });

    test('a failed new-scene cleanup reports only the cleanup failure', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        const created = pb.create(pb.SceneSchema, {
            id: 'B',
            enabled: false,
            kind: {
                case: 'fullscreen',
                value: pb.create(pb.Scene_FullscreenSchema, {
                    widget: pb.create(pb.WidgetSchema, {
                        id: 'widget-b',
                        config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                    }),
                }),
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            addFullscreenScene: () => ({ value: created.id }),
            getScene: () => ({ scene: created, runningWidgetCount: 56, maxRunningWidgetCount: 56 }),
            removeScene: () => {
                throw new ConnectError('cleanup failed', Code.Internal);
            },
            previewScene: () =>
                (async function* () {
                    yield* [];
                    throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
                })(),
        });
        renderPage();
        await flush();

        fireEvent.click(screen.getByRole('button', { name: 'Add New' }));
        fireEvent.click(screen.getByRole('menuitem', { name: 'Full Screen' }));
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Clock' }));
        await flush();
        await flush();

        expect(document.body.textContent).toContain('cleanup failed');
        expect(document.body.textContent).not.toContain(LIMIT_MESSAGE);
    });

    test('a server-cancelled existing-scene revert reports only the revert failure', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        server[0] = pb.create(pb.SceneSchema, {
            id: 'A',
            enabled: false,
            kind: {
                case: 'fullscreen',
                value: pb.create(pb.Scene_FullscreenSchema, {
                    widget: pb.create(pb.WidgetSchema, {
                        id: 'widget-a',
                        config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                    }),
                }),
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            updateWidget: () => {
                throw new ConnectError('revert cancelled', Code.Canceled);
            },
            previewScene: () => {
                throw new ConnectError('connection lost', Code.Unavailable);
            },
        });
        const { container } = renderPage();
        await flush();

        const edit = container.querySelector<HTMLButtonElement>(`#${ROW_ID_PREFIX}A-edit`);
        if (!edit) throw new Error('scene edit button not rendered');
        fireEvent.click(edit);
        await flush();

        expect(document.body.textContent).toContain('revert cancelled');
        expect(document.body.textContent).not.toContain('Display preview connection lost!');
    });

    test('a rejected fullscreen preview explains the running widget limit', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        server[0] = pb.create(pb.SceneSchema, {
            id: 'A',
            enabled: false,
            kind: {
                case: 'fullscreen',
                value: pb.create(pb.Scene_FullscreenSchema, {
                    widget: pb.create(pb.WidgetSchema, {
                        id: 'widget-a',
                        config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                    }),
                }),
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            previewScene: () =>
                (async function* () {
                    yield* [];
                    throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
                })(),
        });
        const { container } = renderPage();
        await flush();

        const edit = container.querySelector<HTMLButtonElement>(`#${ROW_ID_PREFIX}A-edit`);
        if (!edit) throw new Error('scene edit button not rendered');
        fireEvent.click(edit);
        await flush();

        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
        expect(document.body.textContent).toContain(LIMIT_MESSAGE);
    });

    test('a preview connection failure after admission keeps the editor open', async () => {
        const manifest = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.FULL],
        });
        server[0] = pb.create(pb.SceneSchema, {
            id: 'A',
            enabled: false,
            kind: {
                case: 'fullscreen',
                value: pb.create(pb.Scene_FullscreenSchema, {
                    widget: pb.create(pb.WidgetSchema, {
                        id: 'widget-a',
                        config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                    }),
                }),
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            previewScene: () =>
                (async function* () {
                    yield pb.create(pb.EmptySchema);
                    throw new ConnectError('connection lost', Code.Unavailable);
                })(),
        });
        const { container } = renderPage();
        await flush();

        const edit = container.querySelector<HTMLButtonElement>(`#${ROW_ID_PREFIX}A-edit`);
        if (!edit) throw new Error('scene edit button not rendered');
        fireEvent.click(edit);
        await flush();

        expect(screen.getByRole('dialog', { name: 'Configure Widget' })).toBeTruthy();
        expect(document.body.textContent).toContain('Display preview connection lost!');
    });
});

test('shows running widget capacity reported by the backend', async () => {
    renderPage();
    await flush();

    expect(screen.getByText('Running widgets: 1 / 56')).toBeTruthy();
});
