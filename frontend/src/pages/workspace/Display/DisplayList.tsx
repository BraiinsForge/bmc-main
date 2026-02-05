import { Component, createRef, Fragment } from 'react';
import { cloneDeep, debounce, isPlainObject } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type NavigateFunction, useNavigate } from 'react-router';

// Libs
import { getID } from './const';
import { toast } from '@/lib/toast';
import { listenDocumentEvent } from '@/lib/dom';
import { assertUnreachable, assertUndefined } from '@/lib/ts';
import { setState, Sized, stopEventPropagation } from '@/lib/react';
import { Form, type FormPropsToLocalState, type iField } from '@/lib/form';

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

type FormStateBlockHeight = FormPropsToLocalState<Comp.FormWidgetBlockHeightProps>;
type FormStateBlockchainData = FormPropsToLocalState<Comp.FormWidgetBlockchainDataProps>;
type FormStateBraiinsPool = FormPropsToLocalState<Comp.FormWidgetBraiinsPoolProps>;
type FormStateClock = FormPropsToLocalState<Comp.FormWidgetClockProps>;
type FormStateHalvingCountdown = FormPropsToLocalState<Comp.FormWidgetHalvingCountdownProps>;
type FormStateCountdown = FormPropsToLocalState<Comp.FormWidgetCountdownProps>;
type FormStateRemoteImage = FormPropsToLocalState<Comp.FormWidgetRemoteImageProps>;
type FormStateRemoteWidget = FormPropsToLocalState<Comp.FormWidgetRemoteWidgetProps>;
type FormStateTicker = FormPropsToLocalState<Comp.FormWidgetTickerProps>;

// Can be both edit & create dialogs
type DialogStates = {
    blockHeight: {
        data: FormStateBlockHeight;
        isEdit: boolean;
        sceneID: string;
    };
    blockchainData: {
        data: FormStateBlockchainData;
        isEdit: boolean;
        sceneID: string;
    };
    braiinsPool: {
        data: FormStateBraiinsPool;
        isEdit: boolean;
        sceneID: string;
    };
    clock: {
        data: FormStateClock;
        isEdit: boolean;
        sceneID: string;
    };
    halvingCountdown: {
        data: FormStateHalvingCountdown;
        isEdit: boolean;
        sceneID: string;
    };
    countdown: {
        data: FormStateCountdown;
        isEdit: boolean;
        sceneID: string;
    };
    remoteImage: {
        data: FormStateRemoteImage;
        isEdit: boolean;
        sceneID: string;
    };
    remoteWidget: {
        data: FormStateRemoteWidget;
        isEdit: boolean;
        sceneID: string;
    };
    ticker: {
        data: FormStateTicker;
        isEdit: boolean;
        sceneID: string;
    };
};
function getInitialDialogStates(): DialogStates {
    const getForm = () => ({
        isEdit: false,
        sceneID: '',
        data: {
            errors: null,
            values: {},
        },
    });

    return {
        blockHeight: getForm(),
        blockchainData: getForm(),
        braiinsPool: getForm(),
        clock: getForm(),
        halvingCountdown: getForm(),
        countdown: getForm(),
        remoteImage: getForm(),
        remoteWidget: getForm(),
        ticker: getForm(),
    };
}

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

interface State {
    isLoading: boolean;

    scenes: pb.Scene[];
    accounts: pb.Account[];
    timezones: pb.Timezone[];
    sounds: pb.SoundInfo[];
    recentRemoteWidgets: pb.RemoteWidget[];

    cycle: {
        isOpen: boolean;
        isActive: boolean;
        defaultDurationSeconds: number;
        effect: pb.SceneCyclingTransition;
    };

    openDialogKind: null | 'scene-select' | keyof DialogStates;
    remoteWidgetUrl: {
        value: string;
        errors: null | string[];
    };
    dialogStates: DialogStates;
}
const getInitialState = (): State => ({
    isLoading: false,

    scenes: [],
    accounts: [],
    timezones: [],
    sounds: [],
    recentRemoteWidgets: [],

    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 0,
        effect: pb.SceneCyclingTransition.SLIDE,
    },

    openDialogKind: null,
    remoteWidgetUrl: { value: '', errors: null },
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
        this.#loadRecentRemoteWidgets();
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
            const reqConf = { signal };

            const [{ timezones }, { sceneCycling }, { accounts }, { sounds }] = await Promise.all([
                pb.rpc.sys.getTimezoneList({}, reqConf),
                pb.rpc.scenes.getSceneCycling({}, reqConf),
                pb.rpc.accounts.getAllAccounts({}, reqConf),
                pb.rpc.config.listSounds({}, reqConf),
            ]);
            this.setState(s => ({
                accounts,
                timezones,
                sounds,
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
            msg ||= formatMessage({ defaultMessage: 'Failed to load timezones!' });
            toast.error(msg);
        }
    };

    private abortRecentRemoteWidgetsLoad = pb.abort.get();
    #loadRecentRemoteWidgets = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.abortRecentRemoteWidgetsLoad.replace();
            const { recentRemoteWidgets } = await pb.rpc.scenes.getRecentRemoteWidgets({}, { signal });
            this.setState({ recentRemoteWidgets });
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load recent remote widgets!' });
            toast.error(msg);
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
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            addNew: formatMessage({ defaultMessage: 'Add New' }),
        };
    }

    #sceneAddChooseKind = (): void => {
        this.setState({ openDialogKind: 'scene-select' }, this.#loadRecentRemoteWidgets);
    };
    #sceneAddFullscreen = async (kind: Comp.SceneKind): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { remoteWidgetUrl } = this.state;

        let $kind: pb.WidgetKind['value'];
        let $openDialogKind: NonNullable<State['openDialogKind']>;

        switch (kind) {
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

            case 'blockchainData':
                $openDialogKind = 'blockchainData';
                $kind = { case: 'blockchainData', value: pb.create(pb.BlockchainDataWidgetSchema) };
                break;

            case 'braiinsPool':
                $openDialogKind = 'braiinsPool';
                $kind = { case: 'braiinsPool', value: pb.create(pb.BraiinsPoolWidgetSchema) };
                break;

            case 'remoteImage':
                $openDialogKind = 'remoteImage';
                $kind = { case: 'remoteImage', value: pb.create(pb.RemoteImageWidgetSchema) };
                break;

            case 'halvingCountdown':
                $openDialogKind = 'halvingCountdown';
                $kind = { case: 'halvingCountdown', value: pb.create(pb.HalvingCountdownWidgetSchema) };
                break;

            case 'remoteWidget':
                $openDialogKind = 'remoteWidget';
                $kind = {
                    case: 'remoteWidget',
                    // At the time of writing, the remote widget is the only one
                    // that actually reads these values upon creation, so we
                    // need to provide them and read potential errors.
                    value: pb.create(pb.RemoteWidgetSchema, { widgetUrl: remoteWidgetUrl.value }),
                };
                break;

            case 'countdown':
                $openDialogKind = 'countdown';
                $kind = { case: 'countdown', value: pb.create(pb.CountdownWidgetSchema) };
                break;

            default:
                assertUnreachable(kind, 'Invalid scene kind!');
        }

        // When the widget is created, we get back a scene ID
        // and have to reload the scenes to get the new scene data.
        //
        // This can fail with addition of remote widget
        // because it validates the provided URL.
        let sceneID: string;
        try {
            const res = await pb.rpc.scenes.addFullscreenScene({
                widgetKind: { $typeName: 'braiins.bmc.web.WidgetKind', value: $kind },
            });
            this.#notifySceneAdded();
            sceneID = res.value;
        } catch ($) {
            const e = pb.parseFormErrors<pb.AddFullscreenSceneRequest>($);
            const wke = e.fields.widgetKind as Maybe<Rec>;

            // We either manage to parse out errors for the remote widget URL…
            if (isPlainObject(wke) && 'remoteWidget' in wke && Array.isArray(wke.remoteWidget)) {
                const errors = wke.remoteWidget as string[];
                this.setState(s => ({
                    remoteWidgetUrl: { ...s.remoteWidgetUrl, errors },
                }));
            }

            // …or just notify generically that something has failed
            else {
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to add display widget!' });
                toast.error(msg);
            }

            return;
        }

        // Now that the scene has been added, we need to re-load the scenes
        // to get the new scene data to make sure our state is consistent with the server.
        const scenes = await this.#loadScenes();
        const scene = scenes.find(x => x.id === sceneID);
        const dialogStates = getInitialDialogStates();
        const size = pb.WidgetSize.FULL;

        if (scene?.kind.case === 'fullscreen' && scene.kind.value.widget) {
            const widgetKind = scene.kind.value.widget.kind;
            switch (widgetKind?.value?.case) {
                case undefined:
                    break;

                case 'clock':
                    dialogStates.clock.data = {
                        errors: null,
                        values: Comp.unpackClockWidgetKind(widgetKind, size),
                    };
                    break;

                case 'tickerBtc':
                    dialogStates.ticker.data = {
                        errors: null,
                        values: Comp.unpackTicketWidgetKind(widgetKind, size),
                    };
                    break;

                case 'blockHeight':
                    dialogStates.blockHeight.data = {
                        errors: null,
                        values: Comp.unpackBlockHeightWidgetKind(widgetKind, size),
                    };
                    break;

                case 'blockchainData':
                    dialogStates.blockchainData.data = {
                        errors: null,
                        values: Comp.unpackBlockchainDataWidgetKind(widgetKind, size),
                    };
                    break;

                case 'braiinsPool':
                    dialogStates.braiinsPool.data = {
                        errors: null,
                        values: Comp.unpackBraiinsPoolWidgetKind(widgetKind, size),
                    };
                    break;

                case 'remoteImage':
                    dialogStates.remoteImage.data = {
                        errors: null,
                        values: Comp.unpackRemoteImageWidgetKind(widgetKind, size),
                    };
                    break;

                case 'halvingCountdown':
                    dialogStates.halvingCountdown.data = {
                        errors: null,
                        values: Comp.unpackHalvingCountdownWidgetKind(widgetKind, size),
                    };
                    break;

                case 'remoteWidget':
                    dialogStates.remoteWidget.data = {
                        errors: null,
                        values: Comp.unpackRemoteWidgetKind(widgetKind, size),
                    };
                    break;

                case 'countdown':
                    dialogStates.countdown.data = {
                        errors: null,
                        values: Comp.unpackCountdownWidgetKind(widgetKind, size),
                    };
                    break;

                default:
                    widgetKind?.value && assertUnreachable(widgetKind?.value, 'Unknown widget kind!');
            }
        }

        dialogStates[$openDialogKind].sceneID = sceneID;
        await setState(this, {
            dialogStates,
            openDialogKind: $openDialogKind,
            remoteWidgetUrl: { value: '', errors: null },
        });
        this.#previewOpen(sceneID);
    };
    #sceneAddFullscreenRemote = async (): Promise<void> => {
        this.#sceneAddFullscreen('remoteWidget');
    };
    #sceneAddCombined = async (): Promise<void> => {
        const { navigate } = this.props;

        const response = await pb.rpc.scenes.addCombinedScene({});
        navigate(URLS.pages.display.combined.getHref(response.value), { replace: false });

        this.#notifySceneAdded();
    };

    #openDialogCancel = (): void => {
        this.abortPreview.abort();
        const { openDialogKind, dialogStates } = getInitialState();
        this.setState({
            openDialogKind,
            dialogStates,
            remoteWidgetUrl: { value: '', errors: null },
        });
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
            }, this.#sceneFullscreenWidgetSubmitDebounced);
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

    #sceneFullscreenWidgetSubmit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        const { openDialogKind, dialogStates } = this.state;
        if (!openDialogKind || !(openDialogKind in dialogStates)) {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, cannot submit without open dialog!' }));
            return;
        }

        const data = dialogStates[openDialogKind as keyof DialogStates];
        const scene = this.#getScene(data.sceneID);

        if (!scene) {
            toast.error(formatMessage({ defaultMessage: 'Widget edit: data not found!' }));
            return;
        }
        if (scene.kind.case !== 'fullscreen') {
            toast.error(formatMessage({ defaultMessage: 'Widget edit: not a fullscreen widget, aborting!' }));
            return;
        }

        let widgetKind: pb.WidgetKind;
        switch (openDialogKind) {
            case 'scene-select': {
                toast.error(formatMessage({ defaultMessage: 'Invalid state, cannot submit without open dialog!' }));
                return;
            }

            case 'clock':
                widgetKind = Comp.createClockWidgetKind(dialogStates.clock.data.values);
                break;

            case 'ticker':
                widgetKind = Comp.createTickerWidgetKind(dialogStates.ticker.data.values);
                break;

            case 'blockHeight':
                widgetKind = Comp.createBlockHeightWidgetKind(dialogStates.blockHeight.data.values);
                break;

            case 'blockchainData':
                widgetKind = Comp.createBlockchainDataWidgetKind(dialogStates.blockchainData.data.values);
                break;

            case 'braiinsPool':
                widgetKind = Comp.createBraiinsPoolWidgetKind(dialogStates.braiinsPool.data.values);
                break;

            case 'remoteImage':
                widgetKind = Comp.createRemoteImageWidgetKind(dialogStates.remoteImage.data.values);
                break;

            case 'halvingCountdown':
                widgetKind = Comp.createHalvingCountdownWidgetKind(dialogStates.halvingCountdown.data.values);
                break;

            case 'remoteWidget':
                widgetKind = Comp.createRemoteWidgetKind(dialogStates.remoteWidget.data.values);
                break;

            case 'countdown':
                widgetKind = Comp.createCountdownWidgetKind(dialogStates.countdown.data.values);
                break;

            default:
                assertUnreachable(openDialogKind, 'Submit: Invalid dialog kind!');
        }

        const widget = scene.kind.value.widget;
        if (!widget) {
            toast.error(formatMessage({ defaultMessage: 'Scene edit: no widget value, aborting!' }));
            return;
        }

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
            toast.success(formatMessage({ defaultMessage: 'Widget updated!' }));
        } catch ($) {
            // Parse out form specific errors
            const { global, fields } = pb.parseFormErrors($);

            const res = this.state.dialogStates;
            (Object.keys(dialogStates) as Array<keyof DialogStates>).forEach(key => {
                if (!Object.hasOwn(fields, key)) return;
                res[key].data.errors = { fields: fields[key] };
            });
            this.setState({ dialogStates: res });

            if (global.length) {
                let msg = pb.renderFieldErrorsAsList(global);
                msg ||= formatMessage({ defaultMessage: 'Failed to update widget!' });
                toast.error(msg);
            }
        }

        this.#loadScenesDebounced();
    };
    #sceneFullscreenWidgetSubmitDebounced = debounce(this.#sceneFullscreenWidgetSubmit, 300);
    #sceneRemoteWidgetUrlChange = (url: string): void => {
        this.setState({ remoteWidgetUrl: { value: url, errors: null } });
    };
    #sceneAddRender = (): ReactElement => {
        const {
            openDialogKind,
            remoteWidgetUrl,
            recentRemoteWidgets,
            dialogStates: {
                clock,
                ticker,
                blockHeight,
                blockchainData,
                braiinsPool,
                halvingCountdown,
                countdown,
                remoteImage,
                remoteWidget,
            },
            timezones,
            accounts,
            sounds,
        } = this.state;
        const cancel = this.#openDialogCancel;

        return (
            <Fragment>
                <Comp.FormSceneSelect
                    isOpen={openDialogKind === 'scene-select'}
                    onClose={cancel}
                    onSelection={this.#sceneAddFullscreen}
                    remoteWidgetUrl={{
                        value: remoteWidgetUrl.value,
                        error: pb.renderFieldErrorsAsList(remoteWidgetUrl.errors),
                        disabled: false,
                        onChange: this.#sceneRemoteWidgetUrlChange,
                        onSubmit: this.#sceneAddFullscreenRemote,
                    }}
                    remoteWidgetRecents={recentRemoteWidgets}
                />

                <Comp.FormWidgetClock
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

                <Comp.FormWidgetTicker
                    isOpen={openDialogKind === 'ticker'}
                    isEdit={openDialogKind === 'ticker' && ticker.isEdit}
                    onClose={cancel}
                    error={openDialogKind === 'ticker' ? pb.renderFieldErrorsAsList(ticker.data?.errors?.global) : null}
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    timeFrame={this.#getFormFieldStruct('ticker', 'timeFrame')}
                />

                <Comp.FormWidgetBlockHeight
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

                <Comp.FormWidgetBlockchainData
                    isOpen={openDialogKind === 'blockchainData'}
                    isEdit={openDialogKind === 'blockchainData' && blockchainData.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'blockchainData'
                            ? pb.renderFieldErrorsAsList(blockchainData.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                />

                <Comp.FormWidgetHalvingCountdown
                    isOpen={openDialogKind === 'halvingCountdown'}
                    isEdit={openDialogKind === 'halvingCountdown' && halvingCountdown.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'halvingCountdown'
                            ? pb.renderFieldErrorsAsList(halvingCountdown.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                />

                <Comp.FormWidgetBraiinsPool
                    isOpen={openDialogKind === 'braiinsPool'}
                    isEdit={openDialogKind === 'braiinsPool' && braiinsPool.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'braiinsPool'
                            ? pb.renderFieldErrorsAsList(braiinsPool.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    accountId={{
                        ...this.#getFormFieldStruct('braiinsPool', 'accountId'),
                        options: accounts,
                    }}
                    sceneStyle={this.#getFormFieldStruct('braiinsPool', 'sceneStyle')}
                    timeFrame={this.#getFormFieldStruct('braiinsPool', 'timeFrame')}
                />

                <Comp.FormWidgetRemoteImage
                    isOpen={openDialogKind === 'remoteImage'}
                    isEdit={openDialogKind === 'remoteImage' && remoteImage.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'remoteImage'
                            ? pb.renderFieldErrorsAsList(remoteImage.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    url={this.#getFormFieldStruct('remoteImage', 'url')}
                    refreshDurationSec={this.#getFormFieldStruct('remoteImage', 'refreshDurationSec')}
                />

                <Comp.FormWidgetRemoteWidget
                    isOpen={openDialogKind === 'remoteWidget'}
                    isEdit={openDialogKind === 'remoteWidget' && remoteWidget.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'remoteWidget'
                            ? pb.renderFieldErrorsAsList(remoteWidget.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    url={this.#getFormFieldStruct('remoteWidget', 'url')}
                    name={this.#getFormFieldStruct('remoteWidget', 'name')}
                    params={this.#getFormFieldStruct('remoteWidget', 'params')}
                />

                <Comp.FormWidgetCountdown
                    isOpen={openDialogKind === 'countdown'}
                    isEdit={openDialogKind === 'countdown' && countdown.isEdit}
                    onClose={cancel}
                    error={
                        openDialogKind === 'countdown'
                            ? pb.renderFieldErrorsAsList(countdown.data?.errors?.global)
                            : null
                    }
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
                    label={this.#getFormFieldStruct('countdown', 'label')}
                    targetDate={this.#getFormFieldStruct('countdown', 'targetDate')}
                    targetTime={this.#getFormFieldStruct('countdown', 'targetTime')}
                    backgroundColor={this.#getFormFieldStruct('countdown', 'backgroundColor')}
                    fontStyle={this.#getFormFieldStruct('countdown', 'fontStyle')}
                    ledEnabled={this.#getFormFieldStruct('countdown', 'ledEnabled')}
                    ledEffect={this.#getFormFieldStruct('countdown', 'ledEffect')}
                    ledColorR={this.#getFormFieldStruct('countdown', 'ledColorR')}
                    ledColorG={this.#getFormFieldStruct('countdown', 'ledColorG')}
                    ledColorB={this.#getFormFieldStruct('countdown', 'ledColorB')}
                    soundEnabled={this.#getFormFieldStruct('countdown', 'soundEnabled')}
                    soundId={this.#getFormFieldStruct('countdown', 'soundId')}
                    soundVolume={this.#getFormFieldStruct('countdown', 'soundVolume')}
                    soundOptions={sounds}
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
            msg ||= formatMessage({ defaultMessage: 'Failed to move the widget!' });
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

            // Submit to backend
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
            // Design declares that this action does not need a success notification
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

            pb.rpc.scenes.removeScene({ value: id }, { signal });
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
                    // Second push for matched scene
                    if (x.id === id) res.push(x);
                });

                return { scenes: res };
            });

            pb.rpc.scenes.cloneScene({ value: id }, { signal });
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
                const widgetKind = kind?.value.widget?.kind;
                switch (widgetKind?.value.case) {
                    case undefined:
                        break;

                    case 'clock': {
                        const ds = getInitialDialogStates();
                        ds.clock.sceneID = id;
                        ds.clock.isEdit = true;
                        ds.clock.data.values = Comp.unpackClockWidgetKind(widgetKind, pb.WidgetSize.FULL);

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
                        ds.ticker.data.values = Comp.unpackTicketWidgetKind(widgetKind, pb.WidgetSize.FULL);

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
                        ds.blockHeight.data.values = Comp.unpackBlockHeightWidgetKind(widgetKind, pb.WidgetSize.FULL);

                        this.setState(
                            // Set state
                            { openDialogKind: 'blockHeight', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'blockchainData': {
                        const ds = getInitialDialogStates();
                        ds.blockchainData.sceneID = id;
                        ds.blockchainData.isEdit = true;
                        ds.blockchainData.data.values = Comp.unpackBlockchainDataWidgetKind(
                            widgetKind,
                            pb.WidgetSize.FULL,
                        );

                        this.setState(
                            // Set state
                            { openDialogKind: 'blockchainData', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'braiinsPool': {
                        const ds = getInitialDialogStates();
                        ds.braiinsPool.sceneID = id;
                        ds.braiinsPool.isEdit = true;
                        ds.braiinsPool.data.values = Comp.unpackBraiinsPoolWidgetKind(widgetKind, pb.WidgetSize.FULL);

                        this.setState(
                            // Set state
                            { openDialogKind: 'braiinsPool', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'remoteImage': {
                        const ds = getInitialDialogStates();
                        ds.remoteImage.sceneID = id;
                        ds.remoteImage.isEdit = true;
                        ds.remoteImage.data.values = Comp.unpackRemoteImageWidgetKind(widgetKind, pb.WidgetSize.FULL);

                        this.setState(
                            // Set state
                            { openDialogKind: 'remoteImage', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'halvingCountdown': {
                        const ds = getInitialDialogStates();
                        ds.halvingCountdown.sceneID = id;
                        ds.halvingCountdown.isEdit = true;
                        ds.halvingCountdown.data.values = Comp.unpackHalvingCountdownWidgetKind(
                            widgetKind,
                            pb.WidgetSize.FULL,
                        );

                        this.setState(
                            // Set state
                            { openDialogKind: 'halvingCountdown', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'remoteWidget': {
                        const ds = getInitialDialogStates();
                        ds.remoteWidget.sceneID = id;
                        ds.remoteWidget.isEdit = true;
                        ds.remoteWidget.data.values = Comp.unpackRemoteWidgetKind(widgetKind, pb.WidgetSize.FULL);

                        this.setState(
                            // Set state
                            { openDialogKind: 'remoteWidget', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    case 'countdown': {
                        const ds = getInitialDialogStates();
                        ds.countdown.sceneID = id;
                        ds.countdown.isEdit = true;
                        ds.countdown.data.values = Comp.unpackCountdownWidgetKind(widgetKind, pb.WidgetSize.FULL);

                        this.setState(
                            // Set state
                            { openDialogKind: 'countdown', dialogStates: ds },
                            // ...and open the dialog
                            () => this.#previewOpen(id),
                        );

                        break;
                    }

                    default: {
                        assertUndefined(widgetKind?.value, 'Invalid widget kind!');
                    }
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
    const { cycle, duration, /* transitionEffect */ render } = props;
    const intl = useIntl();

    const { formatMessage } = intl;
    const txt = {
        enableCycling: formatMessage({ defaultMessage: 'Enable Screen Cycling' }),
        on: formatMessage({ defaultMessage: 'On' }),
        off: formatMessage({ defaultMessage: 'Off' }),

        defaultDuration: formatMessage({ defaultMessage: 'Default Display Duration' }),
        // txEffect: formatMessage({ defaultMessage: 'Transition Effect' }),
        title: formatMessage({ defaultMessage: 'Screen Cycling' }),
    };

    /**
     * CDS expects specific children types in some places and passes down props that they then use in the child.
     * One example for all is children of menus where they get some handlers.
     *
     * Here, it would however produce errors as form passes everthing it does not consume down to the form element.
     * The extra function wrapper makes sure that no props are passed down to the form element.
     */
    const Content = (): ReactElement => {
        return (
            <Form
                className={css.screenCycleForm}
                // Prevents click events from bubbling up to the dropdown menu and closing it.
                onClick={stopEventPropagation}
            >
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

                {/* <Dropdown<pb.SceneCyclingTransition>
                    id={$('cycle-effect')}
                    label={txt.txEffect}
                    titleText={txt.txEffect}
                    items={pb.sceneCyclingEffectOptions}
                    onChange={x => (x.selectedItem ? transitionEffect.onChange?.(x.selectedItem) : null)}
                    selectedItem={transitionEffect.value ?? undefined}
                    itemToString={x => pb.sceneCyclingEffectToString(intl, x) ?? 'N/A'}
                    renderSelectedItem={x => pb.sceneCyclingEffectToString(intl, x) ?? 'N/A'}
                /> */}
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
