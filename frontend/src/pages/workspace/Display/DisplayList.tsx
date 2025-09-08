import { Component, createRef, Fragment } from 'react';
import { cloneDeep, debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type NavigateFunction, useNavigate } from 'react-router';

// Libs
import { getID } from './const';
import { setState, Sized } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';
import { listenDocumentEvent } from '@/lib/dom';
import { Form, type FormPropsToLocalState, type iField } from '@/lib/form';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Button } from '@/components';
import { Dropdown, OverflowMenu, Toggle } from '@carbon/react';
import {
    Add as IconAdd,
    CarouselHorizontal as IconCycle,
    ChevronDown as IconChevronDown,
    ChevronUp as IconChevronUp,
} from '@carbon/react/icons';
import {
    createBlockHeightWidgetKind,
    createClockWidgetKind,
    createTickerWidgetKind,
    FormSceneSelect,
    FormWidgetBlockHeight,
    type FormWidgetBlockHeightProps,
    FormWidgetClock,
    type FormWidgetClockProps,
    FormWidgetTicker,
    type FormWidgetTickerProps,
    type SceneKind,
    SceneOverviewList,
} from './components';

// Styles
import css from './DisplayList.scss';

const $ = getID('list').get;

type FormStateClock = FormPropsToLocalState<FormWidgetClockProps>;
type FormStateTicker = FormPropsToLocalState<FormWidgetTickerProps>;
type FormStateBlockHeight = FormPropsToLocalState<FormWidgetBlockHeightProps>;

// Can be both edit & create dialogs
type DialogStates = {
    clock: {
        data: FormStateClock;
        isEdit: boolean;
        sceneID: string;
    };
    ticker: {
        data: FormStateTicker;
        isEdit: boolean;
        sceneID: string;
    };
    blockHeight: {
        data: FormStateBlockHeight;
        isEdit: boolean;
        sceneID: string;
    };
};
function getInitialDialogStates(): DialogStates {
    return {
        clock: {
            isEdit: false,
            sceneID: '',
            data: {
                errors: null,
                values: {
                    widgetSize: pb.WidgetSize.FULL,
                    clockStyle: pb.ClockWidget_ClockStyle.ANALOG_ROUND,
                    fontStyle: pb.FontStyle.LIGHT,
                    showDate: true,
                    showSeconds: true,
                    showTimezone: true,
                    timezone: undefined,
                },
            },
        },
        ticker: {
            isEdit: false,
            sceneID: '',
            data: {
                errors: null,
                values: {
                    widgetSize: pb.WidgetSize.FULL,
                    timeFrame: pb.TickerBtcWidget_TimeFrame.DAY_1,
                },
            },
        },
        blockHeight: {
            isEdit: false,
            sceneID: '',
            data: {
                errors: null,
                values: {
                    showDate: true,
                    fontStyle: pb.FontStyle.MEDIUM,
                },
            },
        },
    };
}

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

interface State {
    isLoading: boolean;

    timezones: pb.Timezone[];
    scenes: pb.Scene[];

    cycle: {
        isOpen: boolean;
        isActive: boolean;
        defaultDurationSeconds: number;
        effect: pb.SceneCyclingTransition;
    };

    openDialogKind: null | 'scene-select' | keyof DialogStates;
    dialogStates: DialogStates;
}
const getInitialState = (): State => ({
    isLoading: false,

    timezones: [],
    scenes: [],

    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 0,
        effect: pb.SceneCyclingTransition.SLIDE,
    },

    openDialogKind: null,
    dialogStates: getInitialDialogStates(),
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
    }
    componentWillUnmount() {
        this.#windowClickUnsubscribe();
        pb.abort.all(this);
    }

    private abortLoadMetadata = pb.abort.get();
    #loadMetadata = async (): Promise<void> => {
        const { notify } = this.context;
        const { intl } = this.props;

        try {
            const { signal } = this.abortLoadMetadata.replace();
            const reqConf = { signal };

            const [{ timezones }, { sceneCycling }] = await Promise.all([
                pb.rpc.sys.getTimezoneList({}, reqConf),
                pb.rpc.scenes.getSceneCycling({}, reqConf),
            ]);
            this.setState(s => ({
                timezones,
                cycle: {
                    ...s.cycle,
                    effect: sceneCycling?.transition ?? s.cycle.effect,
                    isActive: sceneCycling?.automaticCyclingEnabled ?? s.cycle.isActive,
                    defaultDurationSeconds: sceneCycling?.automaticCyclingDefaultDurationSec ?? 0,
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string =
                pb.collectAllErrorsAsFormattedList($) ??
                intl.formatMessage({ defaultMessage: 'Failed to load timezones!' });
            notify('error', msg);
        }
    };

    private abortLoadScenes = pb.abort.get();
    #loadScenes = async (): Promise<void> => {
        await setState(this, { isLoading: true });

        try {
            const { signal } = this.abortLoadScenes.replace();
            const { scenes } = await pb.rpc.scenes.getScenes({}, { signal });
            this.setState({ isLoading: false, scenes });
        } catch ($) {
            if (pb.abort.is($)) return;
        }
    };
    #loadScenesDebounced = debounce(this.#loadScenes, 1e3);

    static contextType = AppContext;
    declare context: AppContextType;
    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Display Scenes' }),
            on: formatMessage({ defaultMessage: 'On' }),
            off: formatMessage({ defaultMessage: 'Off' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            addScene: formatMessage({ defaultMessage: 'Add New Scene' }),
        };
    }

    #openDialogSceneSelect = (): void => this.setState({ openDialogKind: 'scene-select' });
    #sceneAddSelectedKind = async (kind: SceneKind): Promise<void> => {
        const { navigate } = this.props;

        let $kind: pb.WidgetKind['value'];
        let $openDialogKind: NonNullable<State['openDialogKind']>;

        switch (kind) {
            case 'combined': {
                const response = await pb.rpc.scenes.addCombinedScene({});
                navigate(URLS.pages.display.combined.getHref(response.value), { replace: false });
                return;
            }

            // Full-screen widgets
            case 'clock':
                $openDialogKind = 'clock';
                $kind = { case: 'clock', value: pb.create(pb.ClockWidgetSchema) };
                break;

            case 'tickerBtc':
                $openDialogKind = 'ticker';
                $kind = { case: 'tickerBtc', value: pb.create(pb.TickerBtcWidgetSchema) };
                break;

            case 'blockHeight':
                $openDialogKind = 'blockHeight';
                $kind = { case: 'blockHeight', value: pb.create(pb.BlockHeightWidgetSchema) };
                break;

            default:
                assertUnreachable(kind, 'Invalid scene kind!');
        }

        const response = await pb.rpc.scenes.addFullscreenScene({
            widgetKind: {
                $typeName: 'braiins.bmc.web.WidgetKind',
                value: $kind,
            },
        });
        const sceneID = response.value;
        await this.#loadScenes();

        await setState(this, s => ({
            openDialogKind: $openDialogKind,
            dialogStates: {
                ...s.dialogStates,
                [$openDialogKind]: {
                    ...s.dialogStates[$openDialogKind],
                    sceneID,
                },
            },
        }));
        this.#previewOpen(sceneID);
    };

    #openDialogCancel = (): void => {
        this.abortPreview.abort();
        const { openDialogKind, dialogStates } = getInitialState();
        this.setState({ openDialogKind, dialogStates });
    };

    #getFormChangeHandler = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['data']['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        return (value: DialogStates[Kind]['data']['values'][FieldKey]) => {
            this.setState(s => {
                const form = cloneDeep(s.dialogStates[widgetKind]);
                form.data = {
                    errors: null,
                    values: {
                        ...form.data.values,
                        [fieldKey]: value,
                    },
                };

                return {
                    dialogStates: {
                        ...s.dialogStates,
                        [widgetKind]: form,
                    },
                };
            }, this.#sceneFullscreenWidgetSubmit);
        };
    };
    #getFormFieldValue = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['data']['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        const { dialogStates } = this.state;

        const values = dialogStates[widgetKind].data.values as DialogStates[Kind]['data']['values'];
        return values?.[fieldKey] ?? null;
    };
    #getFormFieldError = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['data']['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ): null | string => {
        const { dialogStates } = this.state;

        const errors = dialogStates[widgetKind].data.errors as null | pb.FormErrors<any>;
        if (!errors) return null;

        const fieldError = errors.fields?.[fieldKey] as null | pb.FieldErrors;
        return pb.renderFieldErrorsAsList(fieldError);
    };
    #getFormFieldStruct = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['data']['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        return {
            value: this.#getFormFieldValue(widgetKind, fieldKey),
            error: this.#getFormFieldError(widgetKind, fieldKey),
            onChange: this.#getFormChangeHandler(widgetKind, fieldKey),
            disabled: false,
        };
    };

    private abortPreview = pb.abort.get();
    #previewOpen = async (sceneId: string): Promise<void> => {
        const { intl } = this.props;
        const { notify } = this.context;

        try {
            const { signal } = this.abortPreview.replace();
            const stream = pb.rpc.scenes.previewScene({ value: sceneId }, { signal });
            for await (const _ of stream) console.log('💗 Scene preview ping');
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string = intl.formatMessage({ defaultMessage: 'Display preview connection lost!' });
            notify('warning', msg, { id: 'display-preview-lost', timeoutSeconds: 2 });
        }
    };

    #sceneFullscreenWidgetSubmit = async (): Promise<void> => {
        const { notify } = this.context;

        const { openDialogKind, dialogStates } = this.state;
        if (!openDialogKind || !(openDialogKind in dialogStates))
            return notify('error', 'Invalid state, cannot submit without open dialog!');

        const data = dialogStates[openDialogKind as keyof DialogStates];
        const scene = this.#getScene(data.sceneID);

        if (!scene) {
            notify('error', 'Scene edit: cannot find the scene value!');
            return;
        }
        if (scene.kind.case !== 'fullscreen') return notify('error', 'Scene edit: not a fullscreen widget, aborting!');

        let widgetKind: pb.WidgetKind;
        switch (openDialogKind) {
            case 'scene-select':
                return notify('error', 'Invalid state, cannot submit without open dialog!');

            case 'clock':
                widgetKind = createClockWidgetKind(dialogStates.clock.data.values);
                break;

            case 'ticker':
                widgetKind = createTickerWidgetKind(dialogStates.ticker.data.values);
                break;

            case 'blockHeight':
                widgetKind = createBlockHeightWidgetKind(dialogStates.blockHeight.data.values);
                break;

            default:
                assertUnreachable(openDialogKind, 'Submit: Invalid dialog kind!');
        }

        const widget = scene.kind.value.widget;
        if (!widget) return notify('error', 'Scene edit: no widget value, aborting!');

        try {
            const payload = pb.create(pb.UpdateWidgetRequestSchema, {
                id: widget.id,
                sceneId: scene.id,
                kind: widgetKind,
                // These are given for a full-screen widget
                size: pb.WidgetSize.FULL,
                position: { row: 0, col: 0 },
            });
            await pb.rpc.scenes.updateWidget(payload);
            notify('success', 'Widget updated!', { id: 'widget-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            const msg = pb.collectAllErrorsAsFormattedList($) ?? 'Failed to update widget!';
            notify('error', msg);
        }

        this.#loadScenesDebounced();
    };
    #sceneAddRender = (): ReactElement => {
        const {
            openDialogKind,
            dialogStates: { clock, ticker, blockHeight },
            timezones,
        } = this.state;
        const cancel = this.#openDialogCancel;

        return (
            <Fragment>
                <FormSceneSelect
                    variant="scene"
                    isOpen={openDialogKind === 'scene-select'}
                    onClose={cancel}
                    onSelection={this.#sceneAddSelectedKind}
                />

                <FormWidgetClock
                    isOpen={openDialogKind === 'clock'}
                    isEdit={openDialogKind === 'clock' && clock.isEdit}
                    onClose={cancel}
                    error={openDialogKind === 'clock' ? pb.renderFieldErrorsAsList(clock.data.errors?.global) : null}
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    clockStyle={this.#getFormFieldStruct('clock', 'clockStyle')}
                    fontStyle={this.#getFormFieldStruct('clock', 'fontStyle')}
                    showDate={this.#getFormFieldStruct('clock', 'showDate')}
                    showSeconds={this.#getFormFieldStruct('clock', 'showSeconds')}
                    showTimezone={this.#getFormFieldStruct('clock', 'showTimezone')}
                    timezone={{ ...this.#getFormFieldStruct('clock', 'timezone'), options: timezones }}

                    // showWeather={this.#clockGetFieldStruct('clock', 'showWeather')}
                    // weatherLocation={this.#clockGetFieldStruct('clock', 'weatherLocation')}
                />

                <FormWidgetTicker
                    isOpen={openDialogKind === 'ticker'}
                    isEdit={openDialogKind === 'ticker' && ticker.isEdit}
                    onClose={cancel}
                    error={openDialogKind === 'ticker' ? pb.renderFieldErrorsAsList(ticker.data?.errors?.global) : null}
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    timeFrame={this.#getFormFieldStruct('ticker', 'timeFrame')}
                />

                <FormWidgetBlockHeight
                    isOpen={openDialogKind === 'blockHeight'}
                    isEdit={openDialogKind === 'blockHeight' && blockHeight.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'blockHeight'
                            ? pb.renderFieldErrorsAsList(blockHeight.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    showDate={this.#getFormFieldStruct('blockHeight', 'showDate')}
                    fontStyle={this.#getFormFieldStruct('blockHeight', 'fontStyle')}
                />
            </Fragment>
        );
    };

    //
    // /Scene list handlers
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
        const { notify } = this.context;
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
            notify('success', 'Scene moved!', { id: 'scene-move', timeoutSeconds: 1.5 });
        } catch ($) {
            if (pb.abort.is($)) return;
            const message =
                pb.collectAllErrorsAsFormattedList($) || formatMessage({ defaultMessage: 'Failed to move scene!' });
            notify('error', message);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneSetDuration = pb.abort.get();
    #sceneListSetDuration = async (id: string, value: string): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneSetDuration.replace();

        try {
            const cycleDurationSec: undefined | number = value === '' ? undefined : Number.parseInt(value, 10);

            // Optimistic update first
            await setState(this, s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, cycleDurationSec } : x)),
            }));

            pb.rpc.scenes.updateScene(
                {
                    id,
                    enabled: this.#getScene(id)?.enabled ?? true,
                    cycleDurationSec,
                },
                { signal },
            );
            notify('success', 'Scene duration updated!', { id: 'scene-duration-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            if (pb.abort.is($)) return;

            const message =
                pb.collectAllErrorsAsFormattedList($) ||
                formatMessage({ defaultMessage: 'Failed to update scene duration! Please try again!' });
            notify('error', message);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneSetEnabled = pb.abort.get();
    #sceneListSetEnabled = async (id: string, value: boolean): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneSetEnabled.replace();

        try {
            // Optimistic update first
            this.setState(s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, enabled: value } : x)),
            }));

            pb.rpc.scenes.updateScene(
                {
                    id,
                    enabled: value,
                    cycleDurationSec: this.#getScene(id)?.cycleDurationSec ?? 1,
                },
                { signal },
            );
            notify('success', 'Scene state updated!', { id: 'scene-state-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            if (pb.abort.is($)) return;
            const message =
                pb.collectAllErrorsAsFormattedList($) ||
                formatMessage({ defaultMessage: 'Failed to update scene state!!' });
            notify('error', message);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneDelete = pb.abort.get();
    #sceneListDelete = async (id: string): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneDelete.replace();

        try {
            // Optimistic update first
            this.setState(s => ({ scenes: s.scenes.filter(x => x.id !== id) }));

            pb.rpc.scenes.removeScene({ value: id }, { signal });
            notify('success', 'Scene deleted!', { id: 'scene-delete', timeoutSeconds: 1.5 });
        } catch ($) {
            if (pb.abort.is($)) return;
            const message =
                pb.collectAllErrorsAsFormattedList($) || formatMessage({ defaultMessage: 'Failed to delete scene!' });
            notify('error', message);
        }

        this.#loadScenesDebounced();
    };

    private abortSceneClone = pb.abort.get();
    #sceneListClone = async (id: string): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { signal } = this.abortSceneClone.replace();

        try {
            // Optimistic update first
            this.setState(s => {
                const res: pb.Scene[] = [];

                s.scenes.forEach(x => {
                    res.push(x);
                    // Second push for matched scene
                    if (x.id === id) res.push(x);
                });

                return { scenes: res };
            });

            pb.rpc.scenes.cloneScene({ value: id }, { signal });
            notify('success', 'Scene cloned!', { id: 'scene-clone', timeoutSeconds: 1.5 });
        } catch ($) {
            if (pb.abort.is($)) return;
            const message =
                pb.collectAllErrorsAsFormattedList($) || formatMessage({ defaultMessage: 'Failed to clone scene!' });
            notify('error', message);
        }

        this.#loadScenesDebounced();
    };

    #sceneListEdit = (id: string): void => {
        const { navigate } = this.props;
        const scene = this.#getScene(id);
        const kind = scene?.kind;

        switch (kind?.case) {
            case null:
            case undefined:
                break;

            case 'combined': {
                navigate(URLS.pages.display.combined.getHref(id), { replace: false });
                break;
            }

            // fullscreen
            default: {
                const widgetKind = kind?.value.widget?.kind?.value;
                switch (widgetKind?.case) {
                    case undefined:
                        break;

                    case 'clock': {
                        const ds = getInitialDialogStates();
                        ds.clock.sceneID = id;
                        ds.clock.isEdit = true;

                        const v = widgetKind.value;
                        ds.clock.data.values = {
                            clockStyle: v.clockStyle,
                            fontStyle: v.numbersFontStyle,

                            showDate: v.showDate,
                            showSeconds: v.showSeconds,
                            showTimezone: v.showTimezone,

                            timezone: v.timezone,
                        };

                        this.setState(
                            // Set state
                            { openDialogKind: 'clock', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'tickerBtc': {
                        const ds = getInitialDialogStates();
                        ds.ticker.sceneID = id;
                        ds.ticker.isEdit = true;

                        const v = widgetKind.value;
                        ds.ticker.data.values = { timeFrame: v.timeFrame };

                        this.setState(
                            // Set state
                            { openDialogKind: 'ticker', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'blockHeight': {
                        const ds = getInitialDialogStates();
                        ds.blockHeight.sceneID = id;
                        ds.blockHeight.isEdit = true;

                        const v = widgetKind.value;
                        ds.blockHeight.data.values = {
                            showDate: v.showTimestamp,
                            fontStyle: v.numbersFontStyle,
                        };

                        this.setState(
                            // Set state
                            { openDialogKind: 'blockHeight', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    default:
                        assertUnreachable(widgetKind);
                }
                break;
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
        const { notify } = this.context;
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
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to update scene cycling settings!' });
            notify('error', msg);
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
        const { on, off, addScene } = this.#txt;

        const cycleToggleText: string = formatMessage(
            { defaultMessage: 'Screen Cycling: {status}' },
            { status: cycle.isActive ? on : off },
        );

        return (
            <Sized<HTMLDivElement>
                render={(ref, size) => {
                    const iconLayout: boolean = !!size && size.width <= 800;
                    const mobileLayout: boolean = !!size && size.width <= 600;

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
                                                renderIcon={() => (
                                                    <div className={css.screenCycleButtonContent}>
                                                        <IconCycle />
                                                        {iconLayout && !mobileLayout ? null : (
                                                            <span children={cycleToggleText} />
                                                        )}
                                                        {cycle.isOpen ? <IconChevronUp /> : <IconChevronDown />}
                                                    </div>
                                                )}
                                                selectorPrimaryFocus="input,button,select"
                                                size="sm"
                                                children={x.content}
                                            />
                                        </div>
                                    );
                                }}
                            />

                            {iconLayout && !mobileLayout ? (
                                <Button
                                    id={$('add-scene')}
                                    key="add-scene-button"
                                    kind="primary"
                                    onClick={this.#openDialogSceneSelect}
                                    icon={IconAdd}
                                    hasIconOnly
                                    title={addScene}
                                    tooltipPosition="bottom"
                                />
                            ) : (
                                <Button
                                    id={$('add-scene')}
                                    key="add-scene-button"
                                    kind="primary"
                                    onClick={this.#openDialogSceneSelect}
                                    icon={IconAdd}
                                    children={addScene}
                                />
                            )}
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
                            children={intl.formatMessage({ defaultMessage: 'Display Scenes description... ' })}
                        />
                    </div>

                    {this.#headerRender()}
                </header>

                <main>
                    <SceneOverviewList
                        scenes={scenes}
                        onAdd={this.#openDialogSceneSelect}
                        onMove={this.#sceneListMove}
                        onEdit={this.#sceneListEdit}
                        onClone={this.#sceneListClone}
                        onDelete={this.#sceneListDelete}
                        onToggle={this.#sceneListSetEnabled}
                        onDurationChange={this.#sceneListSetDuration}
                        defaultSceneDuration={cycle.defaultDurationSeconds}
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
    const intl = useIntl();
    const { formatMessage } = intl;
    const { cycle, duration, transitionEffect, render } = props;

    const txt = {
        enableCycling: formatMessage({ defaultMessage: 'Enable Screen Cycling' }),
        on: formatMessage({ defaultMessage: 'On' }),
        off: formatMessage({ defaultMessage: 'Off' }),

        defaultDuration: formatMessage({ defaultMessage: 'Default Display Duration' }),
        txEffect: formatMessage({ defaultMessage: 'Transition Effect' }),
    };

    const title: string = formatMessage({ defaultMessage: 'Screen Cycling' });
    /**
     * CDS expects specific children types in some places and passes down props that they then use in the child.
     * One example for all is children of menus where they get some handlers.
     *
     * Here, it would however produce errors as form passes everthing it does not consume down to the form element.
     * The extra function wrapper makes sure that no props are passed down to the form element.
     */
    const Content = (): ReactElement => {
        return (
            <Form className={css.screenCycleForm}>
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
                    onChange={x => (x.selectedItem ? duration.onChange(x.selectedItem) : null)}
                    selectedItem={duration.value ?? undefined}
                    itemToString={pb.sceneCycleDurationToString}
                    renderSelectedItem={pb.sceneCycleDurationToString}
                />

                <Dropdown<pb.SceneCyclingTransition>
                    id={$('cycle-effect')}
                    label={txt.txEffect}
                    titleText={txt.txEffect}
                    items={pb.sceneCyclingEffectOptions}
                    onChange={x => (x.selectedItem ? transitionEffect.onChange(x.selectedItem) : null)}
                    selectedItem={transitionEffect.value ?? undefined}
                    itemToString={x => pb.sceneCyclingEffectToString(intl, x) ?? 'N/A'}
                    renderSelectedItem={x => pb.sceneCyclingEffectToString(intl, x) ?? 'N/A'}
                />
            </Form>
        );
    };

    return render({ title, content: <Content /> });
}

export default function DisplayList() {
    const intl = useIntl();
    const navigate = useNavigate();
    return <View intl={intl} navigate={navigate} />;
}
