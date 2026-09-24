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

import { afterEach, beforeEach, describe, expect, rstest, test } from '@rstest/core';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react/pure';
import { Code, ConnectError } from '@connectrpc/connect';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { IntlProvider } from 'react-intl';
import { MemoryRouter, Route, Routes } from 'react-router';

import DisplayCombined from './DisplayCombined';
import type { ServiceMocks } from '@/lib/proto';
import { deferred } from '@/lib/async';
import { Toaster } from '@/lib/toast';
import * as pb from '@/proto';
import { store } from '@/store';
import { deckCapabilities } from './capabilities.fixture';
import { mocks } from '@/proto/transport';
import { paramDef } from './fn/test-helpers';

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

const LIMIT_ERROR = 'running widget limit exceeded: 56 running, operation would activate 1, maximum 56';
const LIMIT_MESSAGE = 'Running widget limit reached.';
const manifest = pb.create(pb.WidgetManifestSchema, {
    uid: 'clock',
    name: 'Clock',
    supportedSizes: [pb.WidgetSize.SMALL],
});
const scene = pb.create(pb.SceneSchema, {
    id: 'scene-1',
    enabled: true,
    kind: {
        case: 'combined',
        value: pb.create(pb.Scene_CombinedSchema, { widgets: [] }),
    },
});

function installMocks(): void {
    mocks.clear();
    registerMocks(pb.services.SceneManagementService, {
        getScene: () => ({ scene, runningWidgetCount: 56, maxRunningWidgetCount: 56 }),
        getAvailableWidgets: () => ({ widgets: [manifest] }),
        previewScene: () => (async function* () {})(),
        addWidget: () => {
            throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
        },
    });
    store.setHardwareCapabilities(deckCapabilities({ combinedScenesSupported: true }));
    registerMocks(pb.services.AccountManagementService, { getAllAccounts: () => ({ accounts: [] }) });
    registerMocks(pb.services.CredentialManagementService, { getCredentialTypes: () => ({ credentialTypes: [] }) });
    registerMocks(pb.services.SystemService, { getTimezoneList: () => ({ timezones: [] }) });
}

// Spelled out rather than composed with `getID`, so a change to the id scheme
// fails here instead of being silently followed.
const WIDGET_ID_PREFIX = 'bmc-display-comp-combined-scene-widget-';
const PICKER_MODAL_ID = 'bmc-display-comp-scene-select-kind-modal';
const MANIFEST_DONE_ID = 'bmc-display-comp-manifest-form-done';
const MANIFEST_MODAL_ID = 'bmc-display-comp-manifest-form-dialog';
const WIDGET_1_EDIT_ID = `${WIDGET_ID_PREFIX}widget-1-edit`;
const COUNT_INPUT_ID = 'bmc-display-comp-manifest-form-param-count';

function elementById(id: string): HTMLElement {
    const el = document.getElementById(id);
    if (!el) throw new Error(`#${id} not rendered`);
    return el;
}

function combinedScene(widgets: pb.Widget[]): pb.Scene {
    return pb.create(pb.SceneSchema, {
        id: 'scene-1',
        enabled: true,
        kind: { case: 'combined', value: pb.create(pb.Scene_CombinedSchema, { widgets }) },
    });
}

// Placeholder slots carry generated ids, so the first free one is matched by shape.
function clickAddSlot(container: HTMLElement): void {
    const addButton = container.querySelector<HTMLButtonElement>(`[id^="${WIDGET_ID_PREFIX}"][id$="-add"]`);
    if (!addButton) throw new Error('combined-scene add button not rendered');
    fireEvent.click(addButton);
}

// The page footer carries its own "Done" that navigates away,
// so the manifest editor's has to be reached by id rather than by role.
async function clickManifestDone(): Promise<void> {
    fireEvent.click(await waitFor(() => elementById(MANIFEST_DONE_ID)));
}

// Carbon keeps both dialogs mounted and toggles `is-visible`,
// so presence in the DOM says nothing about which one is open.
function modalIsOpen(id: string): boolean {
    return document.getElementById(id)?.classList.contains('is-visible') ?? false;
}

// These tests assert that an RPC did *not* fire,
// so the pending chain must drain or they would pass by asserting too early.
async function settle(): Promise<void> {
    await act(async () => {});
}

function closePicker(): void {
    const modal = document.getElementById(PICKER_MODAL_ID);
    if (!modal) throw new Error('widget picker not rendered');
    fireEvent.click(within(modal).getByRole('button', { name: /close/i }));
}

function closeManifestEditor(): void {
    fireEvent.click(within(elementById(MANIFEST_MODAL_ID)).getByRole('button', { name: /close/i }));
}

function renderPage() {
    return render(
        <HelmetProvider>
            <IntlProvider locale="en">
                <MemoryRouter initialEntries={['/display/scene-1']}>
                    <Routes>
                        <Route path="/display/:id" element={<DisplayCombined />} />
                    </Routes>
                    <Toaster />
                </MemoryRouter>
            </IntlProvider>
        </HelmetProvider>,
    );
}

beforeEach(installMocks);

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.useRealTimers();
    store.setHardwareCapabilities(null);
});

describe('running widget limit', () => {
    test('an accepted preview refreshes the running widget counter', async () => {
        rstest.useFakeTimers();
        let getSceneCalls = 0;
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({
                scene,
                runningWidgetCount: getSceneCalls++ === 0 ? 50 : 56,
                maxRunningWidgetCount: 56,
            }),
            previewScene: () =>
                (async function* () {
                    yield pb.create(pb.EmptySchema);
                })(),
        });

        renderPage();
        await act(async () => {
            await rstest.runAllTimersAsync();
        });
        // The preview continuation settles after the first drain, then schedules the debounced reload.
        await act(async () => {
            await rstest.runAllTimersAsync();
        });

        expect(screen.getByText('Running widgets: 56 / 56')).toBeTruthy();
    });

    test('a rejected preview explains the running widget limit', async () => {
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({
                scene: { ...scene, enabled: false },
                runningWidgetCount: 56,
                maxRunningWidgetCount: 56,
            }),
            previewScene: () =>
                (async function* () {
                    yield* [];
                    throw new ConnectError(LIMIT_ERROR, Code.ResourceExhausted);
                })(),
        });

        renderPage();

        await waitFor(() => expect(document.body.textContent).toContain(LIMIT_MESSAGE));
        expect(screen.queryByText('Edit Combined Scene')).toBeNull();
    });

    test('a preview connection failure after admission keeps the editor open', async () => {
        registerMocks(pb.services.SceneManagementService, {
            previewScene: () =>
                (async function* () {
                    yield pb.create(pb.EmptySchema);
                    throw new ConnectError('connection lost', Code.Unavailable);
                })(),
        });

        renderPage();

        await waitFor(() => expect(document.body.textContent).toContain('Display preview connection lost!'));
        expect(screen.getByText('Edit Combined Scene')).toBeTruthy();
    });

    // Adding is offered while a slot still looks free, so the server can still refuse:
    // the count moves between load and click, or another client takes the last slot.
    test('a rejected combined add does not open the widget editor and explains the running widget limit', async () => {
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({ scene, runningWidgetCount: 55, maxRunningWidgetCount: 56 }),
        });
        const { container } = renderPage();

        await screen.findByText('Running widgets: 55 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));

        await waitFor(() => expect(document.body.textContent).toContain(LIMIT_MESSAGE));
        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
    });
});

describe('dialog session lifecycle', () => {
    let stored: pb.Widget[];
    let removedWidgetIds: string[];

    beforeEach(() => {
        stored = [];
        removedWidgetIds = [];
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({
                scene: combinedScene(stored),
                runningWidgetCount: stored.length,
                maxRunningWidgetCount: 56,
            }),
            getAvailableWidgets: () => ({ widgets: [manifest] }),
            previewScene: () => (async function* () {})(),
            addWidget: ({ req }) => {
                const widget = pb.create(pb.WidgetSchema, {
                    id: `widget-${stored.length + 1}`,
                    position: req.position,
                    size: req.size,
                    config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                });
                stored.push(widget);
                return { value: widget.id };
            },
            updateWidget: () => ({}),
            removeWidget: ({ req }) => {
                removedWidgetIds.push(req.id);
                stored = stored.filter(w => w.id !== req.id);
                return {};
            },
        });
    });

    // The editor opens against the placement the server chose, so a widget the read-back
    // does not carry cannot be configured against a guess — it would only be refused later.
    test('a widget missing from the read-back reports itself instead of opening the editor', async () => {
        registerMocks(pb.services.SceneManagementService, {
            addWidget: () => ({ value: 'never-stored' }),
        });
        const { container } = renderPage();

        await screen.findByText('Running widgets: 0 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));

        await waitFor(() =>
            expect(document.body.textContent).toContain('Widget added, but its settings could not be opened.'),
        );
        expect(screen.queryByRole('dialog', { name: 'Configure Widget' })).toBeNull();
    });

    test('cancelling a newly added widget removes it again', async () => {
        const { container } = renderPage();

        await screen.findByText('Running widgets: 0 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));
        await waitFor(() => expect(modalIsOpen(MANIFEST_MODAL_ID)).toBe(true));
        closeManifestEditor();
        await settle();

        expect(removedWidgetIds).toEqual(['widget-1']);
        expect(stored).toEqual([]);
    });

    test('cancelling a newly added widget removes it only after a preview still in flight', async () => {
        const counted = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.SMALL],
            params: [paramDef('paramInteger', 'count')],
        });
        const previewHeld = deferred<void>();
        let previewRequested = false;
        const applied: string[] = [];
        registerMocks(pb.services.SceneManagementService, {
            getAvailableWidgets: () => ({ widgets: [counted] }),
            updateWidget: async () => {
                previewRequested = true;
                await previewHeld;
                applied.push('update');
                return {};
            },
            removeWidget: () => {
                applied.push('remove');
                return {};
            },
        });
        const { container } = renderPage();

        await screen.findByText('Running widgets: 0 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));
        fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), { target: { value: '8' } });
        await waitFor(() => expect(previewRequested).toBe(true));
        closeManifestEditor();
        await settle();
        previewHeld.resolve();

        await waitFor(() => expect(applied).toHaveLength(2));
        expect(applied).toEqual(['update', 'remove']);
    });

    test('closing the picker after saving a widget leaves that widget in place', async () => {
        const { container } = renderPage();

        await screen.findByText('Running widgets: 0 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));
        await clickManifestDone();
        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));

        clickAddSlot(container);
        await waitFor(() => expect(modalIsOpen(PICKER_MODAL_ID)).toBe(true));
        closePicker();
        await waitFor(() => expect(modalIsOpen(PICKER_MODAL_ID)).toBe(false));
        await settle();

        expect(removedWidgetIds).toEqual([]);
        expect(stored.map(w => w.id)).toEqual(['widget-1']);
    });

    function refuseSecondAdd(refusal: ConnectError): void {
        let addCalls = 0;
        registerMocks(pb.services.SceneManagementService, {
            addWidget: ({ req }) => {
                addCalls += 1;
                if (addCalls > 1) throw refusal;
                const widget = pb.create(pb.WidgetSchema, {
                    id: 'widget-1',
                    position: req.position,
                    size: req.size,
                    config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
                });
                stored.push(widget);
                return { value: widget.id };
            },
        });
    }

    async function addSaveThenRefuse(container: HTMLElement, refusalText: string): Promise<void> {
        await screen.findByText('Running widgets: 0 / 56');
        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));
        await clickManifestDone();
        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));

        clickAddSlot(container);
        fireEvent.click(await screen.findByRole('button', { name: /Clock/ }));
        await waitFor(() => expect(document.body.textContent).toContain(refusalText));

        closePicker();
        await waitFor(() => expect(modalIsOpen(PICKER_MODAL_ID)).toBe(false));
        await settle();
    }

    test('closing the picker after a refused second add leaves the saved widget in place', async () => {
        refuseSecondAdd(new ConnectError(LIMIT_ERROR, Code.ResourceExhausted));
        const { container } = renderPage();

        await addSaveThenRefuse(container, LIMIT_MESSAGE);

        expect(removedWidgetIds).toEqual([]);
        expect(stored.map(w => w.id)).toEqual(['widget-1']);
    });

    // The cleanup path never reads the error code,
    // so one refusal other than the capacity limit stands in for all of them.
    test('closing the picker after a refusal that is not the capacity limit keeps the widget', async () => {
        refuseSecondAdd(new ConnectError('size is not supported', Code.FailedPrecondition));
        const { container } = renderPage();

        await addSaveThenRefuse(container, 'size is not supported');

        expect(removedWidgetIds).toEqual([]);
        expect(stored.map(w => w.id)).toEqual(['widget-1']);
    });

    test('closing the picker after saving an edit does not revert it', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        stored.push(
            pb.create(pb.WidgetSchema, {
                id: 'widget-1',
                position: pb.create(pb.WidgetPositionSchema, { row: 0, col: 0 }),
                size: pb.WidgetSize.SMALL,
                config: pb.create(pb.WidgetConfigSchema, { widgetUid: manifest.uid }),
            }),
        );
        registerMocks(pb.services.SceneManagementService, {
            updateWidget: ({ req }) => {
                updates.push(req);
                return {};
            },
        });

        const { container } = renderPage();

        await screen.findByText('Running widgets: 1 / 56');
        fireEvent.click(await waitFor(() => elementById(WIDGET_1_EDIT_ID)));
        await clickManifestDone();
        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));
        const updatesAfterSave = updates.length;

        clickAddSlot(container);
        await waitFor(() => expect(modalIsOpen(PICKER_MODAL_ID)).toBe(true));
        closePicker();
        await waitFor(() => expect(modalIsOpen(PICKER_MODAL_ID)).toBe(false));
        await settle();

        expect(updates.length).toBe(updatesAfterSave);
    });
});

describe('cancelling an edit', () => {
    // A size change relocates the widget when the new span will not fit in place:
    // MEDIUM spans two columns, so one parked in the last column shifts left.
    test('restores the position that a cancelled size change moved the widget from', async () => {
        const resizable = pb.create(pb.WidgetManifestSchema, {
            uid: 'clock',
            name: 'Clock',
            supportedSizes: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM],
            params: [paramDef('paramInteger', 'count')],
        });
        const widget = pb.create(pb.WidgetSchema, {
            id: 'widget-1',
            position: pb.create(pb.WidgetPositionSchema, { row: 0, col: 3 }),
            size: pb.WidgetSize.SMALL,
            config: pb.create(pb.WidgetConfigSchema, {
                widgetUid: resizable.uid,
                params: pb.create(pb.WidgetDataStructSchema, {
                    fields: {
                        count: pb.create(pb.WidgetDataValueSchema, { kind: { case: 'integerValue', value: 7 } }),
                    },
                }),
            }),
        });
        const updates: pb.UpdateWidgetRequest[] = [];
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({ scene: combinedScene([widget]), runningWidgetCount: 1, maxRunningWidgetCount: 56 }),
            getAvailableWidgets: () => ({ widgets: [resizable] }),
            previewScene: () => (async function* () {})(),
            updateWidget: ({ req }) => {
                updates.push(req);
                return {};
            },
        });

        renderPage();

        await screen.findByText('Running widgets: 1 / 56');
        fireEvent.click(await waitFor(() => elementById(WIDGET_1_EDIT_ID)));
        fireEvent.click(await screen.findByRole('button', { name: 'Medium' }));
        closeManifestEditor();
        // Cancelling cancels the live-preview debounce, so the revert is the only write.
        await waitFor(() => expect(updates).toHaveLength(1));

        const [revert] = updates;
        expect(revert.size).toBe(pb.WidgetSize.SMALL);
        expect(revert.position?.col).toBe(3);
        expect(revert.params?.fields.count?.kind).toEqual({ case: 'integerValue', value: 7 });
    });
});

describe('editing a placed widget', () => {
    type UpdateWidgetMock = ServiceMocks<typeof pb.services.SceneManagementService>['updateWidget'];

    const pooled = pb.create(pb.WidgetManifestSchema, {
        uid: 'pool-stats',
        name: 'Pool Stats',
        supportedSizes: [pb.WidgetSize.SMALL],
        params: [paramDef('paramInteger', 'count')],
        credentials: [
            pb.create(pb.CredentialSlotDefinitionSchema, { key: 'pool', typeId: 'braiins-pool', label: 'Pool' }),
        ],
    });
    function mockServer(updateWidget: UpdateWidgetMock, boundAccountId = 'pool-a'): void {
        const widget = pb.create(pb.WidgetSchema, {
            id: 'widget-1',
            position: pb.create(pb.WidgetPositionSchema, { row: 0, col: 0 }),
            size: pb.WidgetSize.SMALL,
            config: {
                widgetUid: pooled.uid,
                params: { fields: { count: { kind: { case: 'integerValue', value: 7 } } } },
                credentialBindings: { bindings: { pool: boundAccountId } },
            },
        });
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({ scene: combinedScene([widget]), runningWidgetCount: 1, maxRunningWidgetCount: 56 }),
            getAvailableWidgets: () => ({ widgets: [pooled] }),
            updateWidget,
        });
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({
                accounts: [
                    ...['a', 'b'].map(x =>
                        pb.create(pb.AccountSchema, {
                            id: `pool-${x}`,
                            name: `Pool ${x.toUpperCase()}`,
                            typeId: 'braiins-pool',
                        }),
                    ),
                    pb.create(pb.AccountSchema, { id: 'token-1', name: 'Some Token', typeId: 'generic-token' }),
                ],
            }),
        });
    }

    async function openEditor(): Promise<void> {
        renderPage();
        await screen.findByText('Running widgets: 1 / 56');
        fireEvent.click(await waitFor(() => elementById(WIDGET_1_EDIT_ID)));
    }

    const countOf = (req: pb.UpdateWidgetRequest) => {
        const kind = req.params?.fields.count?.kind;
        return kind?.case === 'integerValue' ? kind.value : undefined;
    };

    async function pickAccount(name: string): Promise<void> {
        fireEvent.click(within(elementById(MANIFEST_MODAL_ID)).getByRole('combobox'));
        const option = await waitFor(() => {
            const found = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(o =>
                o.textContent?.includes(name),
            );
            if (!found) throw new Error(`${name} not offered`);
            return found;
        });
        fireEvent.click(option);
    }

    test('picking an account previews it on the widget', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');

        await waitFor(() => expect(updates.at(-1)?.credentialBindings?.bindings).toEqual({ pool: 'pool-b' }));
    });

    test('Cancel restores the account the widget opened with', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');
        await waitFor(() => expect(updates).toHaveLength(1));
        closeManifestEditor();

        await waitFor(() => expect(updates).toHaveLength(2));
        expect(updates[1].credentialBindings?.bindings).toEqual({ pool: 'pool-a' });
    });

    test('Cancel leaves out an original account deleted while the dialog was open', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');
        await waitFor(() => expect(updates).toHaveLength(1));
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({
                accounts: [pb.create(pb.AccountSchema, { id: 'pool-b', name: 'Pool B', typeId: 'braiins-pool' })],
            }),
        });
        closeManifestEditor();

        await waitFor(() => expect(updates).toHaveLength(2));
        expect(updates[1].credentialBindings?.bindings).toEqual({});
    });

    test('Cancel falls back to the loaded accounts when the re-sync fails', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');
        await waitFor(() => expect(updates).toHaveLength(1));
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => {
                throw new ConnectError('connection lost', Code.Unavailable);
            },
        });
        closeManifestEditor();

        await waitFor(() => expect(updates).toHaveLength(2));
        expect(updates[1].credentialBindings?.bindings).toEqual({ pool: 'pool-a' });
    });

    test('Done leaves out an account deleted while the dialog was open', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: () => ({
                accounts: [pb.create(pb.AccountSchema, { id: 'pool-a', name: 'Pool A', typeId: 'braiins-pool' })],
            }),
        });
        await clickManifestDone();

        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));
        expect(updates.at(-1)?.credentialBindings?.bindings).toEqual({});
    });

    test('an account of the wrong type keeps account changes out of the preview and Cancel', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        }, 'token-1');

        await openEditor();
        await pickAccount('Pool B');
        await waitFor(() => expect(updates).toHaveLength(1));
        closeManifestEditor();

        await waitFor(() => expect(updates).toHaveLength(2));
        expect(updates.map(u => u.credentialBindings)).toEqual([undefined, undefined]);
    });

    test('until the accounts load, the picker says so instead of judging the binding', async () => {
        mockServer(() => ({}));
        registerMocks(pb.services.AccountManagementService, {
            getAllAccounts: ({ conf }) => {
                conf({ delay: 300 });
                return { accounts: [] };
            },
        });

        await openEditor();

        await waitFor(() => expect(document.body.textContent).toContain('Accounts not loaded'));
        await waitFor(() => expect(document.body.textContent).not.toContain('Accounts not loaded'));
    });

    test('Done leaves untouched bindings to the server', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await clickManifestDone();

        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));
        expect(updates.map(u => u.credentialBindings)).toEqual([undefined]);
    });

    test('the account on show stays while the saved dialog animates closed', async () => {
        mockServer(() => ({}));
        // A closing modal is `aria-hidden`, so the picker in it is only reachable as a hidden role.
        const picker = () => within(elementById(MANIFEST_MODAL_ID)).getByRole('combobox', { hidden: true });

        await openEditor();
        await waitFor(() => expect(picker().textContent).toContain('Pool A'));
        await clickManifestDone();

        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));
        expect(picker().textContent).toContain('Pool A');
    });

    test('Done sends a changed account', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        await pickAccount('Pool B');
        await clickManifestDone();

        await waitFor(() => expect(document.body.textContent).toContain('Widget updated!'));
        expect(updates.at(-1)?.credentialBindings?.bindings).toEqual({ pool: 'pool-b' });
    });

    test('an edit that never touches an account leaves the bindings to the server', async () => {
        const updates: pb.UpdateWidgetRequest[] = [];
        mockServer(({ req }) => {
            updates.push(req);
            return {};
        });

        await openEditor();
        fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), {
            target: { value: '8' },
        });
        await waitFor(() => expect(updates).toHaveLength(1));
        closeManifestEditor();

        await waitFor(() => expect(updates).toHaveLength(2));
        expect(updates.map(u => u.credentialBindings)).toEqual([undefined, undefined]);
    });

    describe('write ordering', () => {
        test('Done lands after a preview still in flight', async () => {
            const previewHeld = deferred<void>();
            const requested: Array<number | undefined> = [];
            const applied: Array<number | undefined> = [];
            mockServer(async ({ req }) => {
                requested.push(countOf(req));
                if (countOf(req) === 8) await previewHeld;
                applied.push(countOf(req));
                return {};
            });

            await openEditor();
            const count = await waitFor(() => elementById(COUNT_INPUT_ID));
            fireEvent.change(count, { target: { value: '8' } });
            await waitFor(() => expect(requested).toEqual([8]));
            fireEvent.change(count, { target: { value: '9' } });
            await clickManifestDone();
            await settle();
            previewHeld.resolve();

            await waitFor(() => expect(applied).toHaveLength(2));
            expect(applied).toEqual([8, 9]);
        });

        test('a timed-out write tells the user it timed out', async () => {
            mockServer(() => {
                throw new ConnectError('deadline exceeded', Code.DeadlineExceeded);
            });

            await openEditor();
            fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), { target: { value: '8' } });

            await waitFor(() => expect(document.body.textContent).toContain("The device didn't answer in time."));
        });

        test('a widget added right after Cancel waits for the writes before it', async () => {
            const previewHeld = deferred<void>();
            const requested: Array<number | undefined> = [];
            const applied: string[] = [];
            mockServer(async ({ req }) => {
                requested.push(countOf(req));
                if (countOf(req) === 8) await previewHeld;
                applied.push(`update ${countOf(req)}`);
                return {};
            });
            registerMocks(pb.services.SceneManagementService, {
                addWidget: () => {
                    applied.push('add');
                    return { value: 'widget-2' };
                },
            });

            await openEditor();
            fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), { target: { value: '8' } });
            await waitFor(() => expect(requested).toEqual([8]));
            closeManifestEditor();
            await settle();
            clickAddSlot(document.body);
            fireEvent.click(await screen.findByRole('button', { name: /Pool Stats/ }));
            await settle();
            previewHeld.resolve();

            await waitFor(() => expect(applied).toHaveLength(3));
            expect(applied).toEqual(['update 8', 'update 7', 'add']);
        });

        test('a widget removed right after Cancel waits for the writes before it', async () => {
            const previewHeld = deferred<void>();
            const requested: Array<number | undefined> = [];
            const applied: string[] = [];
            mockServer(async ({ req }) => {
                requested.push(countOf(req));
                if (countOf(req) === 8) await previewHeld;
                applied.push(`update ${countOf(req)}`);
                return {};
            });
            registerMocks(pb.services.SceneManagementService, {
                removeWidget: () => {
                    applied.push('remove');
                    return {};
                },
            });

            await openEditor();
            fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), { target: { value: '8' } });
            await waitFor(() => expect(requested).toEqual([8]));
            closeManifestEditor();
            await settle();
            fireEvent.click(elementById(`${WIDGET_ID_PREFIX}widget-1-delete`));
            await settle();
            previewHeld.resolve();

            await waitFor(() => expect(applied).toHaveLength(3));
            expect(applied).toEqual(['update 8', 'update 7', 'remove']);
        });

        // The server applies the slow preview last unless Cancel waits its turn.
        // A param edit keeps the revert free of the account re-sync, whose own delay would mask the race.
        test('Cancel lands after a preview still in flight', async () => {
            const previewHeld = deferred<void>();
            const requested: Array<number | undefined> = [];
            const applied: Array<number | undefined> = [];
            mockServer(async ({ req }) => {
                requested.push(countOf(req));
                if (countOf(req) === 8) await previewHeld;
                applied.push(countOf(req));
                return {};
            });

            await openEditor();
            fireEvent.change(await waitFor(() => elementById(COUNT_INPUT_ID)), { target: { value: '8' } });
            await waitFor(() => expect(requested).toEqual([8]));
            closeManifestEditor();
            await settle();
            previewHeld.resolve();

            await waitFor(() => expect(applied).toHaveLength(2));
            expect(applied).toEqual([8, 7]);
        });
    });
});

describe('the add control at capacity', () => {
    const SLOTS_FULL_TITLE = 'All widget slots are in use';

    function addSlotButton(container: HTMLElement): HTMLButtonElement {
        const btn = container.querySelector<HTMLButtonElement>(`[id^="${WIDGET_ID_PREFIX}"][id$="-add"]`);
        if (!btn) throw new Error('combined-scene add button not rendered');
        return btn;
    }

    test('a full device explains the state and offers no way to add', async () => {
        const { container } = renderPage();

        await screen.findByText('Running widgets: 56 / 56');

        expect(document.body.textContent).toContain(SLOTS_FULL_TITLE);
        expect(addSlotButton(container).hasAttribute('disabled')).toBe(true);
    });

    test('a device with room left says nothing and stays addable', async () => {
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({ scene, runningWidgetCount: 55, maxRunningWidgetCount: 56 }),
        });
        const { container } = renderPage();

        await screen.findByText('Running widgets: 55 / 56');

        expect(document.body.textContent).not.toContain(SLOTS_FULL_TITLE);
        expect(addSlotButton(container).hasAttribute('disabled')).toBe(false);
    });

    // Zero is what a missing `max_running_widget_count` decodes to,
    // and an unknown limit must not read as a reached one.
    test('an absent limit leaves the editor usable', async () => {
        registerMocks(pb.services.SceneManagementService, {
            getScene: () => ({ scene, runningWidgetCount: 0, maxRunningWidgetCount: 0 }),
        });
        const { container } = renderPage();

        await screen.findByText('Edit Combined Scene');

        expect(document.body.textContent).not.toContain(SLOTS_FULL_TITLE);
        expect(addSlotButton(container).hasAttribute('disabled')).toBe(false);
    });
});
