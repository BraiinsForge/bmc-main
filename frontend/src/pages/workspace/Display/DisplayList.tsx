import { Component, createRef, Fragment } from 'react';
import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type NavigateFunction, useNavigate } from 'react-router';

// Libs
import * as fn from './fn';
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

type OpenDialogKind = null | 'scene-select' | 'manifest';

interface ManifestFormState {
    manifest: null | pb.WidgetManifest;
    sceneID: string;
    widgetID: string;
    params: Record<string, pb.WidgetDataValue>;
    isNewScene: boolean;
}

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

interface State {
    isLoading: boolean;

    scenes: pb.Scene[];
    manifestWidgets: pb.WidgetManifest[];
    manifestLookup: pb.ManifestLookup;
    manifestsLoading: boolean;
    timezones: pb.Timezone[];

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
    manifestWidgets: [],
    manifestLookup: new Map(),
    manifestsLoading: false,
    timezones: [],

    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 0,
        effect: pb.SceneCyclingTransition.SLIDE,
    },

    openDialogKind: null,
    manifestForm: { manifest: null, sceneID: '', widgetID: '', params: {}, isNewScene: false },
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
    }
    componentWillUnmount() {
        this.#windowClickUnsubscribe();
        pb.abort.all(this);
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
            const { scenes } = await pb.rpc.scenes.getScenes({}, { signal });
            this.setState({ isLoading: false, scenes });
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

            const params = fn.widgetParamsToFormState(widget.config?.params);

            this.setState({
                openDialogKind: 'manifest',
                manifestForm: { manifest, sceneID, widgetID: widget.id, params, isNewScene: true },
            });
            this.#loadScenesDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
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

    #openDialogCancel = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { manifestForm } = this.state;
        this.abortPreview.abort();
        this.setState({ openDialogKind: null });

        if (manifestForm.isNewScene && manifestForm.sceneID) {
            try {
                await pb.rpc.scenes.removeScene({ value: manifestForm.sceneID });
                this.#loadScenesDebounced();
            } catch ($) {
                if (pb.abort.is($)) return;
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to remove widget!' });
                toast.error(msg);
            }
        }
    };

    private abortPreview = pb.abort.get();
    #previewOpen = async (sceneId: string): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortPreview.replace();
            const stream = pb.rpc.scenes.previewScene({ value: sceneId }, { signal });
            for await (const _ of stream) console.log('💗 Scene preview ping');
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string = formatMessage({ defaultMessage: 'Display preview connection lost!' });
            toast.error(msg);
        }
    };

    #handleManifestParamChange = (key: string, value: pb.WidgetDataValue | undefined): void => {
        this.setState(s => {
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
        });
    };

    #handleManifestFormDone = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { manifestForm } = this.state;
        const { manifest, sceneID, widgetID, params } = manifestForm;

        if (!manifest || !widgetID) {
            this.setState({ openDialogKind: null });
            return;
        }

        try {
            const scene = this.#getScene(sceneID);
            const widget = scene?.kind.case === 'fullscreen' ? scene.kind.value.widget : undefined;

            await pb.rpc.scenes.updateWidget({
                id: widgetID,
                sceneId: sceneID,
                position: widget?.position ?? pb.create(pb.WidgetPositionSchema),
                size: widget?.size ?? pb.WidgetSize.FULL,
                params: fn.formStateToWidgetDataStruct(manifest, params),
            });

            this.abortPreview.abort();
            this.setState({ openDialogKind: null });
            this.#loadScenesDebounced();
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update widget!' });
            toast.error(msg);
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
                    error={null}
                    manifest={this.state.manifestForm.manifest}
                    params={this.state.manifestForm.params}
                    onParamChange={this.#handleManifestParamChange}
                    timezones={this.state.timezones}
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

        try {
            // Optimistic update first
            this.setState(s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, enabled: value } : x)),
            }));

            await pb.rpc.scenes.updateScene(
                {
                    id,
                    enabled: value,
                    cycleDurationSec: this.#getScene(id)?.cycleDurationSec,
                },
                { signal },
            );
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
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

        try {
            // Optimistic update first
            this.setState(s => {
                const res: pb.Scene[] = [];

                s.scenes.forEach(x => {
                    res.push(x);
                    if (x.id === id) res.push(x);
                });

                return { scenes: res };
            });

            await pb.rpc.scenes.cloneScene({ value: id }, { signal });
            toast.success(formatMessage({ defaultMessage: 'Widget cloned!' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
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

                const params = fn.widgetParamsToFormState(widget.config?.params);

                this.setState(
                    {
                        openDialogKind: 'manifest',
                        manifestForm: { manifest, sceneID: id, widgetID: widget.id, params, isNewScene: false },
                    },
                    () => this.#previewOpen(id),
                );
                return;
            }
        }
    };

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
                                <MenuItem
                                    label={formatMessage({ defaultMessage: 'Combined Scene' })}
                                    className={css.addMenuButton}
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
        const { scenes, cycle } = this.state;

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

interface ScreenCyclingConfigFormProps {
    cycle: iField<boolean>;
    duration: iField<number>;
    transitionEffect: iField<pb.SceneCyclingTransition>;
    render(x: { title: string; content: ReactElement }): ReactElement;
}
function ScreenCyclingConfigForm(props: ScreenCyclingConfigFormProps): ReactElement {
    const { cycle, duration, render } = props;
    const intl = useIntl();

    const { formatMessage } = intl;
    const txt = {
        enableCycling: formatMessage({ defaultMessage: 'Enable Screen Cycling' }),
        on: formatMessage({ defaultMessage: 'On' }),
        off: formatMessage({ defaultMessage: 'Off' }),

        defaultDuration: formatMessage({ defaultMessage: 'Default Display Duration' }),
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
