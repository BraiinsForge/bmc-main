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

import { Component, createRef, Fragment, type ReactElement } from 'react';
import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type NavigateFunction, useNavigate } from 'react-router';

// Libs
import * as fn from './fn';
import type { FormifiedParams, FormifiedValue, ParamsFormErrors } from './fn';
import { getID } from './const';
import { toast } from '@/lib/toast';
import { listenDocumentEvent } from '@/lib/dom';
import { setState, Sized, stopEventPropagation } from '@/lib/react';
import { Form, type iField } from '@/lib/form';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Dropdown, OverflowMenu, MenuButton, MenuItem, Toggle, Layer } from '@carbon/react';
import {
    CarouselHorizontal as IconCycle,
    ChevronDown as IconChevronDown,
    ChevronUp as IconChevronUp,
    type CarbonIconType,
} from '@carbon/react/icons';
import * as Comp from './components';

// Styles
import css from './DisplayList.scss';

const $ = getID('list').get;

// Monotonic source of unique ids for optimistic clone placeholders.
let optimisticCloneSeq = 0;

type OpenDialogKind = null | 'scene-select' | 'manifest';

enum DialogCloseResult {
    Closed = 'closed',
    CleanupFailed = 'cleanup-failed',
}

interface ManifestFormState {
    manifest: null | pb.WidgetManifest;
    sceneID: string;
    widgetID: string;
    params: FormifiedParams;
    errors: null | ParamsFormErrors;
    isNewScene: boolean;
    originalParams: FormifiedParams;
    credentialBindings: Record<string, string>;
}

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

interface State {
    isLoading: boolean;

    scenes: pb.Scene[];
    runningWidgetCount: number;
    maxRunningWidgetCount: number;
    manifestWidgets: pb.WidgetManifest[];
    manifestLookup: pb.ManifestLookup;
    manifestsLoading: boolean;
    timezones: pb.Timezone[];
    hardwareCapabilities: null | pb.HardwareCapabilities;
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;

    cycle: {
        isOpen: boolean;
        isActive: boolean;
        defaultDurationSeconds: number;
        effect: pb.SceneCyclingTransition;
    };

    openDialogKind: OpenDialogKind;
    manifestForm: ManifestFormState;
}

const getInitialState = (): State => ({
    isLoading: false,

    scenes: [],
    runningWidgetCount: 0,
    maxRunningWidgetCount: 0,
    manifestWidgets: [],
    manifestLookup: new Map(),
    manifestsLoading: false,
    timezones: [],
    hardwareCapabilities: null,
    accounts: [],
    credentialTypes: new Map(),

    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 0,
        effect: pb.SceneCyclingTransition.SLIDE,
    },

    openDialogKind: null,
    manifestForm: {
        manifest: null,
        sceneID: '',
        widgetID: '',
        params: {},
        errors: null,
        isNewScene: false,
        originalParams: {},
        credentialBindings: {},
    },
});

class View extends Component<Props, State> {
    readonly state = getInitialState();

    #cyclePopOverRef = createRef<null | HTMLDivElement>();
    #windowClickHandle = (e: PointerEvent): void => {
        const { cycle } = this.state;
        const clickTarget = e.target as HTMLElement;
        const catchTarget = this.#cyclePopOverRef.current as Maybe<HTMLElement>;

        if (!cycle.isOpen) return;
        if (!clickTarget || !catchTarget) return;
        if (catchTarget.contains(clickTarget)) return;

        this.#cycleDialogClose();
    };
    #windowClickUnsubscribe = (): void => {};

    componentDidMount() {
        const { unsubscribe } = listenDocumentEvent({ name: 'click', handler: this.#windowClickHandle });
        this.#windowClickUnsubscribe = unsubscribe;
        this.#loadMetadata();
        this.#loadScenes();
        this.#loadManifestWidgets();
        this.#loadTimezones();
        this.#loadHardwareCapabilities();
        this.#loadAccounts();
        this.#loadCredentialTypes();
    }
    componentWillUnmount() {
        this.#windowClickUnsubscribe();
        pb.abort.all(this);
        this.#liveUpdateWidget.cancel();
    }

    #notifySuccessDebounced = debounce(toast.success, 1e3);
    #notifySceneAdded = () => {
        const { formatMessage } = this.props.intl;
        toast.success(formatMessage({ defaultMessage: 'Display widget has been successfully added.' }), {
            title: formatMessage({ defaultMessage: 'Display Widget Added' }),
        });
    };

    private abortLoadMetadata = pb.abort.get();
    #loadMetadata = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortLoadMetadata.replace();
            const { sceneCycling } = await pb.rpc.scenes.getSceneCycling({}, { signal });
            this.setState(s => ({
                cycle: {
                    ...s.cycle,
                    effect: sceneCycling?.transition ?? s.cycle.effect,
                    isActive: sceneCycling?.automaticCyclingEnabled ?? s.cycle.isActive,
                    defaultDurationSeconds: sceneCycling?.automaticCyclingDefaultDurationSec ?? 0,
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load scene cycling settings!' });
            toast.error(msg);
        }
    };

    private abortLoadAccounts = pb.abort.get();
    #loadAccounts = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortLoadAccounts.replace();
            const { accounts } = await pb.rpc.accounts.getAllAccounts({}, { signal });
            this.setState({ accounts });
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load accounts!' });
            toast.error(msg);
        }
    };

    private abortLoadCredentialTypes = pb.abort.get();
    #loadCredentialTypes = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortLoadCredentialTypes.replace();
            const { credentialTypes } = await pb.rpc.credentials.getCredentialTypes({}, { signal });
            this.setState({ credentialTypes: new Map(credentialTypes.map(t => [t.id, t])) });
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load credential types!' });
            toast.error(msg);
        }
    };

    private abortLoadHardwareCapabilities = pb.abort.get();
    #loadHardwareCapabilities = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortLoadHardwareCapabilities.replace();
            const hardwareCapabilities = await pb.rpc.hardware.getHardwareCapabilities({}, { signal });
            this.setState({ hardwareCapabilities });
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load hardware capabilities!' });
            toast.error(msg);
        }
    };

    private abortManifestWidgetsLoad = pb.abort.get();
    #loadManifestWidgets = async (): Promise<void> => {
        this.setState({ manifestsLoading: true });
        try {
            const { signal } = this.abortManifestWidgetsLoad.replace();
            const { widgets: manifestWidgets } = await pb.rpc.scenes.getAvailableWidgets({}, { signal });
            const manifestLookup: pb.ManifestLookup = new Map(manifestWidgets.map(m => [m.uid, m]));
            this.setState({ manifestWidgets, manifestLookup, manifestsLoading: false });
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({ manifestsLoading: false });
        }
    };

    private abortLoadTimezones = pb.abort.get();
    #loadTimezones = async (): Promise<void> => {
        try {
            const { signal } = this.abortLoadTimezones.replace();
            const { timezones } = await pb.rpc.sys.getTimezoneList({}, { signal });
            this.setState({ timezones });
        } catch ($) {
            if (pb.abort.is($)) return;
            // Non-fatal: timezone-typed params will fall back to an empty picker.
        }
    };

    private abortLoadScenes = pb.abort.get();
    #loadScenes = async (): Promise<pb.Scene[]> => {
        await setState(this, { isLoading: true });

        try {
            const { signal } = this.abortLoadScenes.replace();
            const { scenes, runningWidgetCount, maxRunningWidgetCount } = await pb.rpc.scenes.getScenes({}, { signal });
            this.setState({ isLoading: false, scenes, runningWidgetCount, maxRunningWidgetCount });
            return scenes;
        } catch ($) {
            if (pb.abort.is($)) return [];
        }

        return [];
    };
    #loadScenesDebounced = debounce(this.#loadScenes, 1e3);

    static contextType = AppContext;
    declare context: AppContextType;
    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Display Widgets' }),
            on: formatMessage({ defaultMessage: 'On' }),
            off: formatMessage({ defaultMessage: 'Off' }),
            addNew: formatMessage({ defaultMessage: 'Add New' }),
        };
    }

    #sceneAddChooseKind = (): void => {
        this.setState({ openDialogKind: 'scene-select' }, () => {
            this.#loadManifestWidgets();
        });
    };

    #sceneAddFullscreenManifest = async (manifest: pb.WidgetManifest): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { value: sceneID } = await pb.rpc.scenes.addFullscreenScene({
                config: {
                    widgetUid: manifest.uid,
                    params: pb.create(pb.WidgetDataStructSchema, { fields: {} }),
                },
            });
            this.#notifySceneAdded();

            const { scene } = await pb.rpc.scenes.getScene({ value: sceneID });
            const widget = scene?.kind.case === 'fullscreen' ? scene.kind.value.widget : undefined;

            if (!widget) {
                this.setState({ openDialogKind: null });
                return;
            }

            const params = fn.widgetParamsToFormifiedState(manifest, widget.config?.params);

            this.setState(
                {
                    openDialogKind: 'manifest',
                    manifestForm: {
                        manifest,
                        sceneID,
                        widgetID: widget.id,
                        params,
                        errors: null,
                        isNewScene: true,
                        originalParams: {},
                        credentialBindings: {},
                    },
                },
                () => this.#previewOpen(sceneID),
            );
            this.#loadScenesDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = fn.runningWidgetLimitErrorMessage($, this.props.intl);
            msg ||= pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to add manifest widget!' });
            toast.error(msg);
        }
    };

    #sceneAddCombined = async (): Promise<void> => {
        const { navigate } = this.props;

        const response = await pb.rpc.scenes.addCombinedScene({});
        navigate(URLS.pages.display.combined.getHref(response.value), { replace: false });

        this.#notifySceneAdded();
    };

    #openDialogCancel = async (): Promise<DialogCloseResult> => {
        const { formatMessage } = this.props.intl;
        const { manifestForm } = this.state;
        const { sceneID, widgetID, manifest, originalParams, isNewScene } = manifestForm;
        this.#liveUpdateWidget.cancel();
        this.abortPreview.abort();
        this.#loadScenesDebounced();
        this.setState({ openDialogKind: null });

        // Cleanup has no client abort signal, so Canceled is a server failure that must be shown.
        if (isNewScene && sceneID) {
            try {
                await pb.rpc.scenes.removeScene({ value: sceneID });
                this.#loadScenesDebounced();
            } catch ($) {
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to remove scene!' });
                toast.error(msg);
                return DialogCloseResult.CleanupFailed;
            }
            return DialogCloseResult.Closed;
        }

        if (!manifest || !widgetID || !sceneID) return DialogCloseResult.Closed;
        const built = fn.buildWidgetDataStruct(manifest, originalParams);
        if (!built.ok) return DialogCloseResult.Closed;
        try {
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: sceneID,
                position: { row: 0, col: 0 },
                size: pb.WidgetSize.FULL,
                params: built.value,
                // Bindings are left out: only params are pushed live, so only params need reverting.
                // Sending them back would re-validate a binding this dialog never touched,
                // and cancelling out of a bad one would then be impossible.
            });
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to revert widget!' });
            toast.error(msg);
            return DialogCloseResult.CleanupFailed;
        }
        return DialogCloseResult.Closed;
    };

    private abortPreview = pb.abort.get();
    #previewOpen = async (sceneId: string): Promise<void> => {
        const { formatMessage } = this.props.intl;
        let previewAccepted = false;

        try {
            const { signal } = this.abortPreview.replace();
            const stream = pb.rpc.scenes.previewScene({ value: sceneId }, { signal });
            for await (const _ of stream) {
                console.log('💗 Scene preview ping');
                if (!previewAccepted) {
                    previewAccepted = true;
                    this.#loadScenesDebounced();
                }
            }
        } catch ($) {
            if (pb.abort.is($)) return;
            if (this.state.manifestForm.sceneID !== sceneId) return;
            const msg = fn.runningWidgetLimitErrorMessage($, this.props.intl);
            if (msg) {
                const closeResult = await this.#openDialogCancel();
                if (closeResult === DialogCloseResult.CleanupFailed) return;
                toast.error(msg);
                return;
            }
            if (!previewAccepted) {
                const closeResult = await this.#openDialogCancel();
                if (closeResult === DialogCloseResult.CleanupFailed) return;
            }
            toast.error(formatMessage({ defaultMessage: 'Display preview connection lost!' }));
        }
    };

    #liveUpdateWidget = debounce(async (): Promise<void> => {
        const { manifestForm } = this.state;
        const { manifest, sceneID, widgetID, params } = manifestForm;
        if (!manifest || !widgetID) return;
        const built = fn.buildWidgetDataStruct(manifest, params);
        if (!built.ok) {
            this.setState(s => ({ manifestForm: { ...s.manifestForm, errors: built.errors } }));
            return;
        }
        const scene = this.#getScene(sceneID);
        const widget = scene?.kind.case === 'fullscreen' ? scene.kind.value.widget : undefined;
        try {
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: sceneID,
                position: widget?.position ?? pb.create(pb.WidgetPositionSchema),
                size: widget?.size ?? pb.WidgetSize.FULL,
                params: built.value,
            });
        } catch ($) {
            if (pb.abort.is($)) return;
            const { formatMessage } = this.props.intl;
            const errors = fn.mapManifestUpdateError($);
            this.setState(s => ({ manifestForm: { ...s.manifestForm, errors } }));

            if (errors.global.length) {
                let msg = pb.renderFieldErrorsAsList(errors.global);
                msg ||= formatMessage({ defaultMessage: 'Failed to update widget!' });
                toast.error(msg);
            }
            return;
        }
        this.setState(s => {
            if (!s.manifestForm.errors) return null;
            return { manifestForm: { ...s.manifestForm, errors: null } };
        });
    }, 300);

    #handleManifestParamChange = (key: string, value: FormifiedValue): void => {
        this.setState(
            s => {
                const def = s.manifestForm.manifest?.params.find(p => p.key === key);
                const errors = def
                    ? fn.revalidateField(s.manifestForm.errors, def, value)
                    : fn.clearFieldError(s.manifestForm.errors, key);
                return {
                    manifestForm: {
                        ...s.manifestForm,
                        params: { ...s.manifestForm.params, [key]: value },
                        errors,
                    },
                };
            },
            () => this.#liveUpdateWidget(),
        );
    };

    #handleCredentialBindingChange = (slotKey: string, accountId: string): void => {
        this.setState(s => {
            const credentialBindings = { ...s.manifestForm.credentialBindings };
            if (accountId) credentialBindings[slotKey] = accountId;
            else delete credentialBindings[slotKey];
            return { manifestForm: { ...s.manifestForm, credentialBindings } };
        });
    };

    #handleManifestFormDone = async (): Promise<void> => {
        const { manifestForm } = this.state;
        const { manifest, sceneID, widgetID, params, credentialBindings } = manifestForm;
        this.#liveUpdateWidget.cancel();

        if (!manifest || !widgetID) {
            this.setState({ openDialogKind: null });
            return;
        }

        const built = fn.buildWidgetDataStruct(manifest, params);
        if (!built.ok) {
            this.setState(s => ({ manifestForm: { ...s.manifestForm, errors: built.errors } }));
            return;
        }

        try {
            const { formatMessage } = this.props.intl;
            const scene = this.#getScene(sceneID);
            const widget = scene?.kind.case === 'fullscreen' ? scene.kind.value.widget : undefined;

            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: sceneID,
                position: widget?.position ?? pb.create(pb.WidgetPositionSchema),
                size: widget?.size ?? pb.WidgetSize.FULL,
                params: built.value,
                credentialBindings: { bindings: credentialBindings },
            });

            toast.success(formatMessage({ defaultMessage: 'Widget updated!' }));
            this.abortPreview.abort();
            this.setState({ openDialogKind: null });
            this.#loadScenesDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;
            const { formatMessage } = this.props.intl;
            const errors = fn.mapManifestUpdateError($);
            this.setState(s => ({ manifestForm: { ...s.manifestForm, errors } }));

            if (errors.global.length) {
                let msg = pb.renderFieldErrorsAsList(errors.global);
                msg ||= formatMessage({ defaultMessage: 'Failed to update widget!' });
                toast.error(msg);
            }
        }
    };

    #sceneAddRender = (): ReactElement => {
        const { openDialogKind } = this.state;

        const fullscreenWidgets = this.state.manifestWidgets.filter(m => m.supportedSizes.includes(pb.WidgetSize.FULL));

        return (
            <Fragment>
                <Comp.FormSceneSelect
                    isOpen={openDialogKind === 'scene-select'}
                    onClose={this.#openDialogCancel}
                    onManifestSelection={this.#sceneAddFullscreenManifest}
                    manifestWidgets={fullscreenWidgets}
                    isLoading={this.state.manifestsLoading}
                />

                <Comp.FormWidgetManifest
                    isOpen={openDialogKind === 'manifest'}
                    onSave={this.#handleManifestFormDone}
                    onCancel={this.#openDialogCancel}
                    manifest={this.state.manifestForm.manifest}
                    params={this.state.manifestForm.params}
                    errors={this.state.manifestForm.errors}
                    onParamChange={this.#handleManifestParamChange}
                    timezones={this.state.timezones}
                    accounts={this.state.accounts}
                    credentialTypes={this.state.credentialTypes}
                    credentialBindings={this.state.manifestForm.credentialBindings}
                    onCredentialBindingChange={this.#handleCredentialBindingChange}
                />
            </Fragment>
        );
    };

    //
    // Scene list handlers
    //

    #getScene = (id: string): null | pb.Scene => {
        return this.state.scenes.find(x => x.id === id) ?? null;
    };

    private abortSceneMove = pb.abort.get();
    #sceneListMove = async (
        scenes: pb.Scene[],
        move: {
            id: string;
            from: number;
            into: number;
        },
    ): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneMove.replace();

        try {
            // Optimistic update first
            this.setState({ scenes });

            await pb.rpc.scenes.moveScene(
                pb.create(pb.MoveSceneRequestSchema, {
                    id: move.id,
                    index: move.into,
                }),
                { signal },
            );
            toast.success(formatMessage({ defaultMessage: 'Widget moved!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to move widget!' });
            toast.error(msg);
        }

        this.#loadScenesDebounced();
    };

    /**
     * This just stores the value to local state and fires of a debounced handler to submit to the backend.
     * This one is split off like that because this values can be changed very quickly by the user by clicking on +/- buttons.
     */
    #sceneListSetDurationLocal = async (id: string, value: string): Promise<void> => {
        const cycleDurationSec: undefined | number = value === '' ? undefined : Number.parseInt(value, 10);

        // Optimistic update first
        this.setState(s => ({ scenes: s.scenes.map(x => (x.id === id ? { ...x, cycleDurationSec } : x)) }));

        // Skip submit when the value violates the proto's >= 1 contract.
        // The input already flags this visually via Carbon's min=1 constraint;
        // no point spamming the backend with rejections.
        if (cycleDurationSec !== undefined && (Number.isNaN(cycleDurationSec) || cycleDurationSec < 1)) {
            return;
        }

        this.#sceneListSetDurationSubmit(id, cycleDurationSec);
    };
    private abortSceneSetDuration = pb.abort.get();
    #sceneListSetDurationSubmit = debounce(async (id: string, valueSeconds: undefined | number): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneSetDuration.replace();

        try {
            const enabled: boolean = this.#getScene(id)?.enabled ?? true;
            await pb.rpc.scenes.updateScene({ id, enabled, cycleDurationSec: valueSeconds }, { signal });
            this.#notifySuccessDebounced(formatMessage({ defaultMessage: 'Widget duration updated!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update widget duration! Please try again!' });
            toast.error(msg);
        }
    }, 500);

    private abortSceneSetEnabled = pb.abort.get();
    #sceneListSetEnabled = async (id: string, value: boolean): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneSetEnabled.replace();

        if (!value) {
            this.setState(s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, enabled: false } : x)),
            }));
        }

        try {
            await pb.rpc.scenes.updateScene(
                {
                    id,
                    enabled: value,
                    // This has to be sent since the RPC does not accept partial updates.
                    // If it's undefined, it means that the default value will be used.
                    cycleDurationSec: this.#getScene(id)?.cycleDurationSec,
                },
                { signal },
            );
            this.setState(s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, enabled: value } : x)),
            }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = fn.runningWidgetLimitErrorMessage($, this.props.intl);
            msg ||= pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update widget state!' });
            toast.error(msg);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneDelete = pb.abort.get();
    #sceneListDelete = async (id: string): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneDelete.replace();

        try {
            // Optimistic update first
            this.setState(s => ({ scenes: s.scenes.filter(x => x.id !== id) }));

            await pb.rpc.scenes.removeScene({ value: id }, { signal });
            toast.success(formatMessage({ defaultMessage: 'Widget deleted!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to delete widget!' });
            toast.error(msg);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneClone = pb.abort.get();
    #sceneListClone = async (id: string): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneClone.replace();

        // Placeholder clone with a unique id (computed out here to keep the
        // updater pure); the debounced reload swaps in the real scene.
        optimisticCloneSeq += 1;
        const optimisticId = pb.optimisticSceneId(optimisticCloneSeq);

        try {
            this.setState(s => {
                const res: pb.Scene[] = [];

                s.scenes.forEach(x => {
                    res.push(x);
                    if (x.id === id) res.push({ ...x, id: optimisticId });
                });

                return { scenes: res };
            });

            await pb.rpc.scenes.cloneScene({ value: id }, { signal });
            toast.success(formatMessage({ defaultMessage: 'Widget cloned!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            this.setState(s => ({ scenes: s.scenes.filter(scene => scene.id !== optimisticId) }));

            let msg = fn.runningWidgetLimitErrorMessage($, this.props.intl);
            msg ||= pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to clone the widget!' });
            toast.error(msg);
        }

        this.#loadScenesDebounced();
    };

    #sceneListEdit = (id: string): void => {
        const { navigate } = this.props;
        const { formatMessage } = this.props.intl;
        const scene = this.#getScene(id);
        const kind = scene?.kind;

        switch (kind?.case) {
            case null:
            case undefined:
                return;

            case 'combined':
                navigate(URLS.pages.display.combined.getHref(id), { replace: false });
                return;

            case 'fullscreen': {
                const widget = kind.value.widget;
                const widgetUid = widget?.config?.widgetUid;
                if (!widgetUid) {
                    toast.error(formatMessage({ defaultMessage: 'Scene has no widget configured.' }));
                    return;
                }

                const manifest = this.state.manifestLookup.get(widgetUid);
                if (!manifest) {
                    toast.error(
                        formatMessage({ defaultMessage: 'Unknown widget type — no matching manifest installed.' }),
                    );
                    return;
                }

                const params = fn.widgetParamsToFormifiedState(manifest, widget.config?.params);
                const bindings = widget.config?.credentialBindings?.bindings ?? {};

                this.setState(
                    {
                        openDialogKind: 'manifest',
                        manifestForm: {
                            manifest,
                            sceneID: id,
                            widgetID: widget.id,
                            params,
                            errors: null,
                            isNewScene: false,
                            originalParams: { ...params },
                            credentialBindings: { ...bindings },
                        },
                    },
                    () => this.#previewOpen(id),
                );
                return;
            }
        }
    };

    //
    // /Scene list handlers
    //

    //
    // Cycle Settings
    //

    #cycleDialogToggle = (open: boolean): void => this.setState(s => ({ cycle: { ...s.cycle, isOpen: open } }));
    #cycleDialogOpen = (): void => this.#cycleDialogToggle(true);
    #cycleDialogClose = (): void => this.#cycleDialogToggle(false);

    private abortCycleSubmit = pb.abort.get();
    #cycleDialogSubmit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { cycle } = this.state;

        try {
            const { signal } = this.abortCycleSubmit.replace();
            await pb.rpc.scenes.setSceneCycling(
                pb.create(pb.SetSceneCyclingRequestSchema, {
                    sceneCycling: {
                        automaticCyclingEnabled: cycle.isActive,
                        automaticCyclingDefaultDurationSec: cycle.defaultDurationSeconds,
                        transition: cycle.effect,
                    },
                }),
                { signal },
            );
            toast.success(formatMessage({ defaultMessage: 'Widget cycling settings updated!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update widget cycling settings!' });
            toast.error(msg);
        } finally {
            this.#loadScenesDebounced();
        }
    };

    #cycleChangeActive = (): void => {
        this.setState(s => ({ cycle: { ...s.cycle, isActive: !s.cycle.isActive } }), this.#cycleDialogSubmit);
    };
    #cycleChangeDuration = (value: number): void => {
        this.setState(s => ({ cycle: { ...s.cycle, defaultDurationSeconds: value } }), this.#cycleDialogSubmit);
    };
    #cycleChangeEffect = (value: pb.SceneCyclingTransition): void => {
        this.setState(s => ({ cycle: { ...s.cycle, effect: value } }), this.#cycleDialogSubmit);
    };

    //
    // /Cycle Settings
    //

    #headerRender = (): ReactElement => {
        const { intl } = this.props;
        const { formatMessage } = intl;

        const { cycle } = this.state;
        const txt = this.#txt;

        const cycleToggleText: string = formatMessage(
            { defaultMessage: 'Widget cycling: {status}' },
            { status: cycle.isActive ? txt.on : txt.off },
        );

        return (
            <Sized<HTMLDivElement>
                render={(ref, size) => {
                    const iconLayout: boolean = !!size && size.width <= 800;
                    const mobileLayout: boolean = !!size && size.width <= 600;
                    const overflowMenuHasLabel: boolean = !(iconLayout && !mobileLayout);

                    return (
                        <div className={css.headerControls} ref={ref}>
                            <ScreenCyclingConfigForm
                                cycle={{ value: cycle.isActive, onChange: this.#cycleChangeActive }}
                                duration={{
                                    value: cycle.defaultDurationSeconds,
                                    onChange: this.#cycleChangeDuration,
                                }}
                                transitionEffect={{ value: cycle.effect, onChange: this.#cycleChangeEffect }}
                                render={x => {
                                    return (
                                        <div className={css.screenCycleButtonWrapper}>
                                            <OverflowMenu
                                                id={$('cycle-form-menu')}
                                                menuOptionsClass={css.screenCycleMenu}
                                                flipped={!mobileLayout}
                                                focusTrap={false}
                                                direction="bottom"
                                                // Sadly we cannot remove this one because we get only three options:
                                                // - leave it on default → "Options"
                                                // - supply our own text
                                                // - get an empty tooltip when giving it an empty string
                                                iconDescription={cycleToggleText}
                                                onOpen={this.#cycleDialogOpen}
                                                onClose={this.#cycleDialogClose}
                                                open={cycle.isOpen}
                                                renderIcon={() => {
                                                    const ToggleIcon: CarbonIconType = cycle.isOpen
                                                        ? IconChevronUp
                                                        : IconChevronDown;

                                                    return (
                                                        <div className={css.screenCycleButtonContent}>
                                                            <IconCycle />
                                                            {overflowMenuHasLabel ? (
                                                                <span children={cycleToggleText} />
                                                            ) : null}
                                                            <ToggleIcon className={css.chevron} />
                                                        </div>
                                                    );
                                                }}
                                                selectorPrimaryFocus="input,button,select"
                                                size="sm"
                                                children={<Layer level={1} children={x.content} />}
                                            />
                                        </div>
                                    );
                                }}
                            />

                            <MenuButton label={txt.addNew} kind="primary" id={$('add-scene')}>
                                <MenuItem
                                    label={formatMessage({ defaultMessage: 'Full Screen' })}
                                    className={css.addMenuButton}
                                    onClick={this.#sceneAddChooseKind}
                                />
                                <CombinedSceneMenuAction
                                    capabilities={this.state.hardwareCapabilities}
                                    label={formatMessage({ defaultMessage: 'Combined Scene' })}
                                    onClick={this.#sceneAddCombined}
                                />
                            </MenuButton>
                        </div>
                    );
                }}
            />
        );
    };

    render() {
        const { intl } = this.props;
        const { scenes, cycle, runningWidgetCount, maxRunningWidgetCount } = this.state;

        return (
            <div className={css.root}>
                <Helmet title={this.#txt.title} />
                <header className={css.header}>
                    <div className={css.headerLeft}>
                        <h1 className={css.title} children={this.#txt.title} />
                        <div
                            className={css.subtitle}
                            children={intl.formatMessage({
                                defaultMessage:
                                    'Configure the content displayed on your Deck. Enable, order, and set durations for each widget to control what’s shown.',
                            })}
                        />
                        {maxRunningWidgetCount > 0 ? (
                            <div
                                className={css.capacity}
                                children={intl.formatMessage(
                                    { defaultMessage: 'Running widgets: {running} / {maximum}' },
                                    { running: runningWidgetCount, maximum: maxRunningWidgetCount },
                                )}
                            />
                        ) : null}
                    </div>

                    {this.#headerRender()}
                </header>

                <main>
                    <Comp.SceneOverviewList
                        scenes={scenes}
                        manifests={this.state.manifestLookup}
                        onMove={this.#sceneListMove}
                        onEdit={this.#sceneListEdit}
                        onClone={this.#sceneListClone}
                        onDelete={this.#sceneListDelete}
                        onToggle={this.#sceneListSetEnabled}
                        onDurationChange={this.#sceneListSetDurationLocal}
                        cycleDefaultDuration={cycle.defaultDurationSeconds}
                        cycleEnabled={!!cycle.isActive}
                    />
                </main>

                {this.#sceneAddRender()}
            </div>
        );
    }
}

export function CombinedSceneMenuAction(props: {
    capabilities: null | pb.HardwareCapabilities;
    label: string;
    onClick: () => void;
}): null | ReactElement {
    if (!fn.combinedSceneAvailable(props.capabilities)) return null;
    return <MenuItem label={props.label} className={css.addMenuButton} onClick={props.onClick} />;
}

interface ScreenCyclingConfigFormProps {
    cycle: iField<boolean>;
    duration: iField<number>;
    transitionEffect: iField<pb.SceneCyclingTransition>;
    render(x: { title: string; content: ReactElement }): ReactElement;
}
function ScreenCyclingConfigForm(props: ScreenCyclingConfigFormProps): ReactElement {
    const { cycle, duration, transitionEffect, render } = props;
    const intl = useIntl();

    const { formatMessage } = intl;
    const txt = {
        enableCycling: formatMessage({ defaultMessage: 'Enable Screen Cycling' }),
        on: formatMessage({ defaultMessage: 'On' }),
        off: formatMessage({ defaultMessage: 'Off' }),

        defaultDuration: formatMessage({ defaultMessage: 'Default Display Duration' }),
        transitionEffect: formatMessage({ defaultMessage: 'Transition Effect' }),
        title: formatMessage({ defaultMessage: 'Screen Cycling' }),
    };

    const Content = (): ReactElement => {
        return (
            <Form className={css.screenCycleForm} onClick={stopEventPropagation}>
                <Toggle
                    id={$('cycle-active')}
                    size="md"
                    toggled={!!cycle.value}
                    onToggle={cycle.onChange}
                    labelText={txt.enableCycling}
                    labelA={txt.off}
                    labelB={txt.on}
                />

                <Dropdown<number>
                    id={$('cycle-duration')}
                    label={txt.defaultDuration}
                    titleText={txt.defaultDuration}
                    items={pb.sceneCycleDurationOptions}
                    onChange={x => (x.selectedItem ? duration.onChange?.(x.selectedItem) : null)}
                    selectedItem={duration.value ?? undefined}
                    itemToString={pb.sceneCycleDurationToString}
                    renderSelectedItem={pb.sceneCycleDurationToString}
                />

                <Dropdown<Exclude<pb.SceneCyclingTransition, 0>>
                    id={$('cycle-transition-effect')}
                    label={txt.transitionEffect}
                    titleText={txt.transitionEffect}
                    items={pb.sceneCyclingEffectOptions}
                    onChange={x => (x.selectedItem ? transitionEffect.onChange?.(x.selectedItem) : null)}
                    selectedItem={transitionEffect.value || undefined}
                    itemToString={x => pb.sceneCyclingEffectToString(intl, x) ?? ''}
                    renderSelectedItem={x => pb.sceneCyclingEffectToString(intl, x) ?? ''}
                />
            </Form>
        );
    };

    return render({ title: txt.title, content: <Content /> });
}

export default function DisplayList() {
    const intl = useIntl();
    const navigate = useNavigate();
    return <View intl={intl} navigate={navigate} />;
}
