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

import { Component, type ReactElement, useEffect } from 'react';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { debounce, cloneDeep } from 'es-toolkit';
import { useParams, useNavigate, type NavigateFunction } from 'react-router';

// Libs
import * as fn from './fn';
import type { FormifiedParams, FormifiedValue, ParamsFormErrors } from './fn';
import { getID } from './const';
import { toast } from '@/lib/toast';
import { delay } from '@/lib/async';
import { setState } from '@/lib/react';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import AppContext, { type AppContextType } from '@/context';

// Components
import * as Comp from './components';
import { ButtonGroup, Button } from '@/components';
import { ChevronLeft as IconChevronLeft } from '@carbon/react/icons';

// Styles
import css from './DisplayCombined.scss';

type OpenDialogKind = null | 'scene-select' | 'manifest';

type CombinedSize = Exclude<pb.WidgetSize, pb.WidgetSize.UNSPECIFIED>;

interface ManifestFormState {
    manifest: null | pb.WidgetManifest;
    widgetID: string;
    params: FormifiedParams;
    errors: null | ParamsFormErrors;
    size: pb.WidgetSize;
    sizeOptions: CombinedSize[];
    position: pb.WidgetPosition;
    anchorPosition: pb.WidgetPosition;
    originalParams: FormifiedParams;
    originalSize: pb.WidgetSize;
    isNewWidget: boolean;
    credentialBindings: Record<string, string>;
}

interface Props {
    navigate: NavigateFunction;
    intl: IntlShape;
    sceneId: string;
}

interface State {
    isLoading: boolean;
    previewRejected: boolean;

    manifestWidgets: pb.WidgetManifest[];
    manifestLookup: pb.ManifestLookup;
    manifestsLoading: boolean;
    scene: null | pb.Scene;
    runningWidgetCount: number;
    maxRunningWidgetCount: number;
    timezones: pb.Timezone[];
    hardwareCapabilities: null | pb.HardwareCapabilities;
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;

    openDialogKind: OpenDialogKind;
    addPosition: null | pb.WidgetPosition;
    manifestForm: ManifestFormState;
}

const getInitialState = (): State => ({
    isLoading: false,
    previewRejected: false,
    manifestWidgets: [],
    manifestLookup: new Map(),
    manifestsLoading: false,
    scene: null,
    runningWidgetCount: 0,
    maxRunningWidgetCount: 0,
    timezones: [],
    hardwareCapabilities: null,
    accounts: [],
    credentialTypes: new Map(),
    openDialogKind: null,
    addPosition: null,
    manifestForm: {
        manifest: null,
        widgetID: '',
        params: {},
        errors: null,
        size: pb.WidgetSize.SMALL,
        sizeOptions: [],
        position: pb.create(pb.WidgetPositionSchema),
        anchorPosition: pb.create(pb.WidgetPositionSchema),
        originalParams: {},
        originalSize: pb.WidgetSize.UNSPECIFIED,
        isNewWidget: false,
        credentialBindings: {},
    },
});

const $ = getID('combined').get;

class View extends Component<Props, State> {
    readonly state = getInitialState();

    static contextType = AppContext;
    declare context: AppContextType;

    #txt = {
        title: this.props.intl.formatMessage({ defaultMessage: 'Edit Combined Scene' }),
    };

    componentDidMount() {
        this.#loadScene();
        this.#previewOpen();
        this.#loadManifestWidgets();
        this.#loadTimezones();
        this.#loadHardwareCapabilities();
        this.#loadAccounts();
        this.#loadCredentialTypes();
    }
    componentWillUnmount() {
        pb.abort.all(this);
        this.#livePreviewWidget.cancel();
        this.#loadSceneDebounced.cancel();
    }

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
            // Silently ignore — manifest widgets are optional
            this.setState({ manifestsLoading: false });
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

    private abortLoadScene = pb.abort.get();
    #loadScene = async (): Promise<void | pb.Scene> => {
        const { sceneId, intl } = this.props;
        await setState(this, { isLoading: true });

        try {
            const { signal } = this.abortLoadScene.replace();
            const { scene, runningWidgetCount, maxRunningWidgetCount } = await pb.rpc.scenes.getScene(
                { value: sceneId },
                { signal },
            );

            this.setState({ isLoading: false, scene: scene || null, runningWidgetCount, maxRunningWidgetCount });
            return scene;
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= intl.formatMessage({ defaultMessage: 'Failed to load scene!' });
            toast.error(msg);
        }
    };
    #loadSceneDebounced = debounce(this.#loadScene, 200);

    private abortPreview = pb.abort.get();
    #previewOpen = async (): Promise<void> => {
        const { sceneId, intl } = this.props;
        let previewAccepted = false;

        try {
            const { signal } = this.abortPreview.replace();
            const stream = pb.rpc.scenes.previewScene({ value: sceneId }, { signal });
            for await (const _ of stream) {
                console.log('💗 Scene preview ping');
                if (!previewAccepted) {
                    previewAccepted = true;
                    this.#loadSceneDebounced();
                }
            }
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg = fn.runningWidgetLimitErrorMessage($, intl);
            if (msg) {
                this.setState({ previewRejected: true }, this.#goBack);
                toast.error(msg);
                return;
            }
            if (!previewAccepted) this.setState({ previewRejected: true }, this.#goBack);
            toast.error(intl.formatMessage({ defaultMessage: 'Display preview connection lost!' }));
        }
    };

    #handleMove: Comp.CombinedSceneViewProps['onWidgetMove'] = async (
        source: pb.Widget,
        target: pb.Widget,
    ): Promise<void> => {
        const { sceneId } = this.props;
        const { formatMessage } = this.props.intl;
        const scene = cloneDeep(this.state.scene);

        if (scene?.kind.case !== 'combined') {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, cannot move widget without combined scene!' }));
            return;
        }
        const widgets = scene.kind.value.widgets;

        // Widget keeps all of its original attributes,
        // but gets a new position from the target slot
        const targetWidgetState: pb.Widget = { ...source, position: target.position };

        // First try positive update
        const canonicalInsertPosition = fn.getWidgetInsertionSlot(widgets, targetWidgetState);

        // If we did not find an insertion slot,
        // there is no point in continuing…
        if (!canonicalInsertPosition) {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, widget seems not to fit!' }));
            return;
        }

        // otherwise update the new widget state with canonical position
        // and do an optimistic update…
        targetWidgetState.position = canonicalInsertPosition;
        this.setState({
            scene: {
                ...scene,
                kind: {
                    case: 'combined',
                    value: {
                        $typeName: 'braiins.bmc.web.Scene.Combined',
                        widgets: widgets
                            // Drop the source widget…
                            .filter(x => x.id !== source.id)
                            // Override the target widget by the updated source
                            .map(x => (x.id === target.id ? targetWidgetState : x)),
                    },
                },
            },
        });

        // send the update to the server
        try {
            await pb.rpc.scenes.updateWidget({
                sceneId,
                id: targetWidgetState.id,
                size: targetWidgetState.size,
                position: targetWidgetState.position,
                params: targetWidgetState.config?.params,
            });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update widget!' });
            toast.error(msg);
        }

        this.#loadSceneDebounced();
    };

    #handleAdd = (position: pb.WidgetPosition): void => {
        this.setState({ openDialogKind: 'scene-select', addPosition: position }, () => {
            this.#loadManifestWidgets();
        });
    };

    /**
     * Sizes the user can pick for a combined-scene widget: the geometry fit at
     * the slot intersected with what the widget's manifest declares it supports.
     */
    #computeSizeOptions(
        manifest: pb.WidgetManifest,
        slot: { id: string; position: pb.WidgetPosition },
    ): CombinedSize[] {
        const scene = this.state.scene;
        const widgets = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];
        const cellFits = fn.getValidWidgetSizes(widgets, slot);
        return cellFits.filter(sz => manifest.supportedSizes.includes(sz)) as CombinedSize[];
    }

    #handleEdit: Comp.CombinedSceneViewProps['onWidgetEdit'] = (id: string): void => {
        const { formatMessage } = this.props.intl;
        const scene = this.state.scene;
        if (scene?.kind.case !== 'combined') {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, cannot edit widget without combined scene!' }));
            return;
        }

        const widget = scene.kind.value.widgets.find(x => x.id === id);
        if (!widget) {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, widget data not found!' }));
            return;
        }

        const manifest = this.state.manifestLookup.get(widget.config?.widgetUid ?? '');
        if (!manifest) {
            toast.error(formatMessage({ defaultMessage: 'Unknown widget type — no matching manifest installed.' }));
            return;
        }

        const params = fn.widgetParamsToFormifiedState(manifest, widget.config?.params);
        const bindings = widget.config?.credentialBindings?.bindings ?? {};
        const position = widget.position ?? pb.create(pb.WidgetPositionSchema);
        const sizeOptions = this.#computeSizeOptions(manifest, { id, position });

        this.setState({
            openDialogKind: 'manifest',
            manifestForm: {
                manifest,
                widgetID: id,
                params,
                errors: null,
                size: widget.size,
                sizeOptions,
                position,
                anchorPosition: position,
                originalParams: { ...params },
                originalSize: widget.size,
                isNewWidget: false,
                credentialBindings: { ...bindings },
            },
        });
    };

    #handleRemove: Comp.CombinedSceneViewProps['onWidgetRemove'] = async (widgetId: string): Promise<void> => {
        const { sceneId } = this.props;

        try {
            // Optimistic update first
            this.setState(s => {
                if (!s.scene) return s;

                const kind = s.scene.kind;
                if (kind?.case !== 'combined') return s;

                kind.value.widgets = kind.value.widgets.filter(x => x.id !== widgetId);

                return {
                    ...s,
                    scene: { ...s.scene, kind },
                };
            });

            await pb.rpc.scenes.removeWidget({ id: widgetId, sceneId });
        } catch ($) {
            if (pb.abort.is($)) return;
        }

        this.#loadSceneDebounced();
    };

    #handleManifestWidgetAdd = async (manifest: pb.WidgetManifest): Promise<void> => {
        const { sceneId, intl } = this.props;
        const { formatMessage } = intl;
        const { addPosition } = this.state;

        const position = addPosition ?? pb.create(pb.WidgetPositionSchema);
        const sizeOptions = this.#computeSizeOptions(manifest, { id: '', position });

        if (sizeOptions.length === 0) {
            toast.error(formatMessage({ defaultMessage: 'This widget has no size that fits this cell.' }));
            return;
        }

        const size = sizeOptions[0];
        const widgets = this.state.scene?.kind.case === 'combined' ? this.state.scene.kind.value.widgets : [];
        const canonicalPosition = fn.getWidgetInsertionSlot(widgets, { id: '', size, position }) ?? position;

        try {
            const { value: newWidgetId } = await pb.rpc.scenes.addWidget({
                sceneId,
                position: canonicalPosition,
                size,
                config: {
                    widgetUid: manifest.uid,
                    params: pb.create(pb.WidgetDataStructSchema, { fields: {} }),
                },
            });

            const { scene } = await pb.rpc.scenes.getScene({ value: sceneId });
            const widget =
                scene?.kind.case === 'combined' ? scene.kind.value.widgets.find(w => w.id === newWidgetId) : undefined;

            const resolvedParams = fn.widgetParamsToFormifiedState(manifest, widget?.config?.params);
            const resolvedPosition = widget?.position ?? canonicalPosition;
            const resolvedSize = widget?.size ?? size;

            this.setState({
                openDialogKind: 'manifest',
                manifestForm: {
                    manifest,
                    widgetID: newWidgetId,
                    params: resolvedParams,
                    errors: null,
                    size: resolvedSize,
                    sizeOptions,
                    position: resolvedPosition,
                    anchorPosition: resolvedPosition,
                    originalParams: {},
                    originalSize: pb.WidgetSize.UNSPECIFIED,
                    isNewWidget: true,
                    credentialBindings: {},
                },
            });
            this.#loadSceneDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = fn.runningWidgetLimitErrorMessage($, this.props.intl);
            msg ||= pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to add widget!' });
            toast.error(msg);
        }
    };

    #livePreviewWidget = debounce(async (): Promise<void> => {
        const { sceneId } = this.props;
        const { widgetID, position, size, params, manifest } = this.state.manifestForm;
        if (!widgetID || !manifest) return;
        const built = fn.buildWidgetDataStruct(manifest, params);
        if (!built.ok) {
            this.setState(s => ({ manifestForm: { ...s.manifestForm, errors: built.errors } }));
            return;
        }
        const paramsStruct = built.value;
        try {
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId,
                position,
                size,
                params: paramsStruct,
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
            const manifestForm = s.manifestForm.errors ? { ...s.manifestForm, errors: null } : s.manifestForm;
            if (s.scene?.kind.case !== 'combined') return { ...s, manifestForm };
            const widgets = s.scene.kind.value.widgets.map(w =>
                w.id === widgetID
                    ? { ...w, size, position, config: w.config ? { ...w.config, params: paramsStruct } : w.config }
                    : w,
            );
            return {
                ...s,
                manifestForm,
                scene: {
                    ...s.scene,
                    kind: {
                        case: 'combined',
                        value: { $typeName: 'braiins.bmc.web.Scene.Combined', widgets },
                    },
                },
            };
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
            () => this.#livePreviewWidget(),
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

    #handleManifestSizeChange = (size: pb.WidgetSize): void => {
        if (this.state.manifestForm.size === size) return;

        const { widgetID, anchorPosition, position } = this.state.manifestForm;
        const scene = this.state.scene;
        const widgets = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];
        const canonicalPosition =
            fn.getWidgetInsertionSlot(widgets, { id: widgetID, size, position: anchorPosition }) ??
            fn.getWidgetInsertionSlot(widgets, { id: widgetID, size, position }) ??
            position;

        this.setState(
            s => ({ manifestForm: { ...s.manifestForm, size, position: canonicalPosition } }),
            () => this.#livePreviewWidget(),
        );
    };

    #handleManifestFormDone = async (): Promise<void> => {
        const { sceneId } = this.props;
        const { manifestForm } = this.state;
        const { manifest, widgetID, params, size, position, credentialBindings } = manifestForm;

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
            this.#livePreviewWidget.cancel();
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId,
                position,
                size,
                params: built.value,
                credentialBindings: { bindings: credentialBindings },
            });

            toast.success(formatMessage({ defaultMessage: 'Widget updated!' }));
            this.setState({ openDialogKind: null, addPosition: null });
            this.#loadSceneDebounced();
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

    #goBack = (): void => {
        this.props.navigate(URLS.pages.display.list);
    };

    #openDialogCancel = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { widgetID, position, originalSize, originalParams, isNewWidget, manifest } = this.state.manifestForm;
        this.setState({ openDialogKind: null, addPosition: null });
        if (!widgetID) return;
        this.#livePreviewWidget.cancel();

        if (isNewWidget) {
            try {
                await pb.rpc.scenes.removeWidget({ id: widgetID, sceneId: this.props.sceneId });
                this.#loadSceneDebounced();
            } catch ($) {
                if (pb.abort.is($)) return;
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to remove widget!' });
                toast.error(msg);
            }
            return;
        }

        if (!manifest) return;
        const built = fn.buildWidgetDataStruct(manifest, originalParams);
        if (!built.ok) return;
        try {
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: this.props.sceneId,
                position,
                size: originalSize,
                params: built.value,
                // Bindings are left out: only params are pushed live, so only params need reverting.
                // Sending them back would re-validate a binding this dialog never touched,
                // and cancelling out of a bad one would then be impossible.
            });
            this.#loadSceneDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to revert widget!' });
            toast.error(msg);
        }
    };

    #delete = async (): Promise<void> => {
        const { intl } = this.props;

        const { scene } = this.state;
        if (!scene) {
            toast.error(intl.formatMessage({ defaultMessage: 'Unknown scene!' }));
            return;
        }

        // Let the user confirm
        const answer: boolean = await this.context.confirm({
            size: 'sm',
            danger: true,
            title: intl.formatMessage({ defaultMessage: 'Delete Scene' }),
            message: intl.formatMessage({ defaultMessage: 'Are you sure you want to delete this scene?' }),
            confirmLabel: intl.formatMessage({ defaultMessage: 'Delete' }),
            cancelLabel: intl.formatMessage({ defaultMessage: 'Cancel' }),
        });
        if (!answer) return;

        try {
            // First we have to stop the preview, because
            // otherwise the backend won't let us delete it
            this.abortPreview.replace();

            // Since the abort is not blocking and/or immediate,
            // we have to wait a bit before backends gets the message
            await delay(600);

            // Now we can delete the scene
            await pb.rpc.scenes.removeScene({ value: scene?.id });
            toast.success(intl.formatMessage({ defaultMessage: 'Scene deleted!' }));
            this.#goBack();
        } catch ($) {
            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= intl.formatMessage({ defaultMessage: 'Failed to delete scene with an unknown error!' });
            toast.error(message);
        }
    };

    render() {
        if (this.state.previewRejected) return null;

        const { intl } = this.props;
        const {
            scene,
            openDialogKind,
            manifestForm,
            manifestWidgets,
            addPosition,
            hardwareCapabilities,
            runningWidgetCount,
            maxRunningWidgetCount,
        } = this.state;

        const widgets: pb.Widget[] = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];
        const manifests = this.state.manifestLookup;

        // When adding, hide widgets whose supported sizes don't intersect what fits at the chosen cell.
        const pickerWidgets = addPosition
            ? manifestWidgets.filter(m => this.#computeSizeOptions(m, { id: '', position: addPosition }).length > 0)
            : manifestWidgets;

        return (
            <CombinedEditorCapabilityGate capabilities={hardwareCapabilities}>
                <div className={css.root}>
                    <Helmet title={this.#txt.title} />
                    <header className={css.header}>
                        <div className={css.headerLeft}>
                            <Button
                                id={$('go-back')}
                                size="md"
                                kind="secondary"
                                onClick={this.#goBack}
                                title={intl.formatMessage({ defaultMessage: 'Back to Scenes List' })}
                                hasIconOnly
                                renderIcon={IconChevronLeft}
                                tooltipAlignment="start"
                                tooltipPosition="bottom"
                            />
                            <h1 className={css.title} children={this.#txt.title} />
                        </div>
                        {maxRunningWidgetCount > 0 ? (
                            <div
                                className={css.capacity}
                                children={intl.formatMessage(
                                    { defaultMessage: 'Running widgets: {running} / {maximum}' },
                                    { running: runningWidgetCount, maximum: maxRunningWidgetCount },
                                )}
                            />
                        ) : null}
                    </header>

                    <main className={css.main}>
                        <p
                            className={css.explainer}
                            children={intl.formatMessage({
                                defaultMessage:
                                    "Drag and drop widgets to organize your screen layout. You'll see a live preview on your device as you make changes. Changes are saved automatically.",
                            })}
                        />

                        <Comp.CombinedSceneView
                            widgets={widgets}
                            manifests={manifests}
                            onWidgetMove={this.#handleMove}
                            onWidgetAdd={this.#handleAdd}
                            onWidgetEdit={this.#handleEdit}
                            onWidgetRemove={this.#handleRemove}
                        />
                        <ButtonGroup spaced className={css.footer}>
                            <Button
                                id={$('done')}
                                children={intl.formatMessage({ defaultMessage: 'Done' })}
                                onClick={this.#goBack}
                            />
                            <Button
                                id={$('delete')}
                                kind="secondary"
                                children={intl.formatMessage({ defaultMessage: 'Delete Scene' })}
                                onClick={this.#delete}
                            />
                        </ButtonGroup>
                    </main>

                    <Comp.FormSceneSelect
                        isOpen={openDialogKind === 'scene-select'}
                        onClose={this.#openDialogCancel}
                        onManifestSelection={this.#handleManifestWidgetAdd}
                        manifestWidgets={pickerWidgets}
                        isLoading={this.state.manifestsLoading}
                    />

                    <Comp.FormWidgetManifest
                        isOpen={openDialogKind === 'manifest'}
                        onSave={this.#handleManifestFormDone}
                        onCancel={this.#openDialogCancel}
                        manifest={manifestForm.manifest}
                        params={manifestForm.params}
                        errors={manifestForm.errors}
                        onParamChange={this.#handleManifestParamChange}
                        timezones={this.state.timezones}
                        size={manifestForm.size}
                        sizeOptions={manifestForm.sizeOptions}
                        onSizeChange={this.#handleManifestSizeChange}
                        accounts={this.state.accounts}
                        credentialTypes={this.state.credentialTypes}
                        credentialBindings={manifestForm.credentialBindings}
                        onCredentialBindingChange={this.#handleCredentialBindingChange}
                    />
                </div>
            </CombinedEditorCapabilityGate>
        );
    }
}

export function CombinedEditorCapabilityGate(props: {
    capabilities: null | pb.HardwareCapabilities;
    children: ReactElement;
}): null | ReactElement {
    const navigate = useNavigate();
    const target = fn.combinedEditorRedirectTarget(props.capabilities);

    useEffect(() => {
        if (target !== null) navigate(target, { replace: true });
    }, [navigate, target]);

    if (props.capabilities === null || target !== null) return null;
    return props.children;
}

export default function DisplayCombined() {
    const intl = useIntl();
    const { id } = useParams();
    const navigate = useNavigate();
    return <View intl={intl} sceneId={id ?? ''} navigate={navigate} />;
}
