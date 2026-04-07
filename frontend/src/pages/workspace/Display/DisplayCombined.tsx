import { Component } from 'react';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { debounce, cloneDeep } from 'es-toolkit';
import { useParams, useNavigate, type NavigateFunction } from 'react-router';

// Libs
import * as fn from './fn';
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
    params: Record<string, string>;
    size: pb.WidgetSize;
    sizeOptions: CombinedSize[];
    position: pb.WidgetPosition;
    // Snapshot for the Cancel handler to revert live-preview RPCs.
    originalParams: Record<string, string>;
    originalSize: pb.WidgetSize;
}

interface Props {
    navigate: NavigateFunction;
    intl: IntlShape;
    sceneId: string;
}

interface State {
    isLoading: boolean;

    manifestWidgets: pb.WidgetManifest[];
    manifestLookup: pb.ManifestLookup;
    manifestsLoading: boolean;
    scene: null | pb.Scene;
    timezones: pb.Timezone[];

    openDialogKind: OpenDialogKind;
    addPosition: null | pb.WidgetPosition;
    manifestForm: ManifestFormState;
}

const getInitialState = (): State => ({
    isLoading: false,
    manifestWidgets: [],
    manifestLookup: new Map(),
    manifestsLoading: false,
    scene: null,
    timezones: [],
    openDialogKind: null,
    addPosition: null,
    manifestForm: {
        manifest: null,
        widgetID: '',
        params: {},
        size: pb.WidgetSize.SMALL,
        sizeOptions: [],
        position: pb.create(pb.WidgetPositionSchema),
        originalParams: {},
        originalSize: pb.WidgetSize.UNSPECIFIED,
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
            const { scene } = await pb.rpc.scenes.getScene({ value: sceneId }, { signal });

            this.setState({ isLoading: false, scene: scene || null });
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

        try {
            const { signal } = this.abortPreview.replace();
            const stream = pb.rpc.scenes.previewScene({ value: sceneId }, { signal });
            for await (const _ of stream) console.log('💗 Scene preview ping');
        } catch ($) {
            if (pb.abort.is($)) return;

            const msg: string = intl.formatMessage({ defaultMessage: 'Display preview connection lost!' });
            toast.error(msg);
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

        const targetWidgetState: pb.Widget = { ...source, position: target.position };

        const canonicalInsertPosition = fn.getWidgetInsertionSlot(widgets, targetWidgetState);
        if (!canonicalInsertPosition) {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, widget seems not to fit!' }));
            return;
        }

        targetWidgetState.position = canonicalInsertPosition;
        this.setState({
            scene: {
                ...scene,
                kind: {
                    case: 'combined',
                    value: {
                        $typeName: 'braiins.bmc.web.Scene.Combined',
                        widgets: widgets
                            .filter(x => x.id !== source.id)
                            .map(x => (x.id === target.id ? targetWidgetState : x)),
                    },
                },
            },
        });

        try {
            // Convert the widget's params into the proto's map<string, string> form.
            const paramsMap: Record<string, string> = {};
            const obj = targetWidgetState.config?.params;
            if (obj && typeof obj === 'object') {
                for (const [k, v] of Object.entries(obj)) {
                    paramsMap[k] = typeof v === 'string' ? v : JSON.stringify(v);
                }
            }

            await pb.rpc.scenes.updateWidget({
                sceneId,
                id: targetWidgetState.id,
                size: targetWidgetState.size,
                position: targetWidgetState.position,
                params: paramsMap,
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

        const params: Record<string, string> = {};
        if (widget.config?.params && typeof widget.config.params === 'object') {
            for (const [k, v] of Object.entries(widget.config.params as Record<string, unknown>)) {
                params[k] = typeof v === 'string' ? v : JSON.stringify(v);
            }
        }

        const position = widget.position ?? pb.create(pb.WidgetPositionSchema);
        const sizeOptions = this.#computeSizeOptions(manifest, { id, position });

        this.setState({
            openDialogKind: 'manifest',
            manifestForm: {
                manifest,
                widgetID: id,
                params,
                size: widget.size,
                sizeOptions,
                position,
                originalParams: { ...params },
                originalSize: widget.size,
            },
        });
    };

    #handleRemove: Comp.CombinedSceneViewProps['onWidgetRemove'] = async (widgetId: string): Promise<void> => {
        const { sceneId } = this.props;

        try {
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

    #handleManifestWidgetAdd = (manifest: pb.WidgetManifest): void => {
        const { formatMessage } = this.props.intl;
        const { addPosition } = this.state;

        const params: Record<string, string> = {};
        for (const def of manifest.params) {
            params[def.key] = def.defaultValue;
        }

        const position = addPosition ?? pb.create(pb.WidgetPositionSchema);
        const sizeOptions = this.#computeSizeOptions(manifest, { id: '', position });

        if (sizeOptions.length === 0) {
            toast.error(formatMessage({ defaultMessage: 'This widget has no size that fits this cell.' }));
            return;
        }

        this.setState({
            openDialogKind: 'manifest',
            manifestForm: {
                manifest,
                widgetID: '',
                params,
                size: sizeOptions[0],
                sizeOptions,
                position,
                originalParams: {},
                originalSize: pb.WidgetSize.UNSPECIFIED,
            },
        });
    };

    #livePreviewWidget = debounce(async (): Promise<void> => {
        const { sceneId } = this.props;
        const { widgetID, position, size, params } = this.state.manifestForm;
        // Add-flow is skipped — respawn-per-keystroke OOMs the GPU.
        if (!widgetID) return;
        try {
            await pb.rpc.scenes.updateWidget({ id: widgetID, sceneId, position, size, params });
        } catch ($) {
            if (pb.abort.is($)) return;
        }
    }, 300);

    #handleManifestParamChange = (key: string, value: string | undefined): void => {
        if (this.state.manifestForm.params[key] === value) return;
        this.setState(
            s => {
                const params = { ...s.manifestForm.params };
                if (value === undefined) {
                    delete params[key];
                } else {
                    params[key] = value;
                }
                return {
                    manifestForm: {
                        ...s.manifestForm,
                        params,
                    },
                };
            },
            () => this.#livePreviewWidget(),
        );
    };

    #handleManifestSizeChange = (size: pb.WidgetSize): void => {
        if (this.state.manifestForm.size === size) return;
        this.setState(
            s => ({ manifestForm: { ...s.manifestForm, size } }),
            () => this.#livePreviewWidget(),
        );
    };

    #handleManifestFormDone = async (): Promise<void> => {
        const { sceneId } = this.props;
        const { formatMessage } = this.props.intl;
        const { manifestForm, addPosition } = this.state;
        const { manifest, widgetID, params, size, position } = manifestForm;

        if (!manifest) {
            this.setState({ openDialogKind: null });
            return;
        }

        try {
            if (widgetID) {
                this.#livePreviewWidget.cancel();
                await pb.rpc.scenes.updateWidget({
                    id: widgetID,
                    sceneId,
                    position,
                    size,
                    params,
                });
            } else if (addPosition) {
                const widgets = this.state.scene?.kind.case === 'combined' ? this.state.scene.kind.value.widgets : [];
                const canonicalPosition =
                    fn.getWidgetInsertionSlot(widgets, { id: '', size, position: addPosition }) ?? addPosition;

                const { value: newWidgetId } = await pb.rpc.scenes.addWidget({
                    sceneId,
                    position: canonicalPosition,
                    size,
                    config: {
                        widgetUid: manifest.uid,
                        params,
                    },
                });

                // Optimistic insert; debounced loadScene reconciles ~200ms later.
                this.setState(s => {
                    if (s.scene?.kind.case !== 'combined') return s;
                    const newWidget = pb.create(pb.WidgetSchema, {
                        id: newWidgetId,
                        position: canonicalPosition,
                        size,
                        config: {
                            widgetUid: manifest.uid,
                            params,
                        },
                    });
                    return {
                        ...s,
                        scene: {
                            ...s.scene,
                            kind: {
                                case: 'combined',
                                value: {
                                    $typeName: 'braiins.bmc.web.Scene.Combined',
                                    widgets: [...s.scene.kind.value.widgets, newWidget],
                                },
                            },
                        },
                    };
                });
            }

            this.setState({ openDialogKind: null, addPosition: null });
            this.#loadSceneDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save widget!' });
            toast.error(msg);
        }
    };

    #goBack = (): void => {
        this.props.navigate(URLS.pages.display.list);
    };

    #openDialogCancel = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { widgetID, position, originalSize, originalParams } = this.state.manifestForm;
        this.setState({ openDialogKind: null });
        if (!widgetID) return;
        this.#livePreviewWidget.cancel();
        try {
            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: this.props.sceneId,
                position,
                size: originalSize,
                params: originalParams,
            });
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
            this.abortPreview.replace();
            await delay(600);

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
        const { intl } = this.props;
        const { scene, openDialogKind, manifestForm, manifestWidgets, addPosition } = this.state;

        const widgets: pb.Widget[] = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];
        const manifests = this.state.manifestLookup;

        // When adding, hide widgets whose supported sizes don't intersect what fits at the chosen cell.
        const pickerWidgets = addPosition
            ? manifestWidgets.filter(m => this.#computeSizeOptions(m, { id: '', position: addPosition }).length > 0)
            : manifestWidgets;

        return (
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
                    error={null}
                    manifest={manifestForm.manifest}
                    params={manifestForm.params}
                    onParamChange={this.#handleManifestParamChange}
                    timezones={this.state.timezones}
                    size={manifestForm.size}
                    sizeOptions={manifestForm.sizeOptions}
                    onSizeChange={this.#handleManifestSizeChange}
                />
            </div>
        );
    }
}

export default function DisplayCombined() {
    const intl = useIntl();
    const { id } = useParams();
    const navigate = useNavigate();
    return <View intl={intl} sceneId={id ?? ''} navigate={navigate} />;
}
