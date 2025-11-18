import { Component } from 'react';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { debounce, cloneDeep, isPlainObject } from 'es-toolkit';
import { useParams, useNavigate, type NavigateFunction } from 'react-router';

// Libs
import * as fn from './fn';
import { getID } from './const';
import { toast } from '@/lib/toast';
import { delay } from '@/lib/async';
import { setState } from '@/lib/react';
import type { FormPropsToLocalState } from '@/lib/form';
import { assertUnreachable, assertUndefined } from '@/lib/ts';

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
type FormDialogState<Data> = {
    data: Data;
    isEdit: boolean;
    widgetID: string;
    position: pb.WidgetPosition;
};

type DialogStates = {
    blockHeight: FormDialogState<FormStateBlockHeight>;
    blockchainData: FormDialogState<FormStateBlockchainData>;
    braiinsPool: FormDialogState<FormStateBraiinsPool>;
    clock: FormDialogState<FormStateClock>;
    halvingCountdown: FormDialogState<FormStateHalvingCountdown>;
    countdown: FormDialogState<FormStateCountdown>;
    remoteImage: FormDialogState<FormStateRemoteImage>;
    remoteWidget: FormDialogState<FormStateRemoteWidget>;
    ticker: FormDialogState<FormStateTicker>;
};
function getInitialDialogStates(): DialogStates {
    const getForm = () => ({
        isEdit: false,
        widgetID: '',
        position: pb.create(pb.WidgetPositionSchema),
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
    navigate: NavigateFunction;
    intl: IntlShape;
    sceneId: string;
}

interface State {
    isLoading: boolean;

    accounts: pb.Account[];
    timezones: pb.Timezone[];
    sounds: pb.SoundInfo[];
    recentRemoteWidgets: pb.RemoteWidget[];
    scene: null | pb.Scene;

    openDialogKind: null | 'scene-select' | keyof DialogStates;
    addPosition: null | pb.WidgetPosition;
    dialogStates: DialogStates;
    remoteWidgetUrl: {
        value: string;
        errors: null | string[];
    };
}
const getInitialState = (): State => ({
    isLoading: false,

    accounts: [],
    timezones: [],
    sounds: [],
    recentRemoteWidgets: [],
    scene: null,

    openDialogKind: null,
    addPosition: null,
    dialogStates: getInitialDialogStates(),
    remoteWidgetUrl: { value: '', errors: null },
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
        this.#loadMetadata();
        this.#previewOpen();
        this.#loadRecentRemoteWidgets();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortLoadMetadata = pb.abort.get();
    #loadMetadata = async (): Promise<void> => {
        const { intl } = this.props;

        try {
            const { signal } = this.abortLoadMetadata.replace();
            const reqConf = { signal };
            const [{ timezones }, { accounts }, { sounds }] = await Promise.all([
                pb.rpc.sys.getTimezoneList({}, reqConf),
                pb.rpc.accounts.getAllAccounts({}, reqConf),
                pb.rpc.config.listSounds({}, reqConf),
            ]);
            this.setState({ timezones, accounts, sounds });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= intl.formatMessage({ defaultMessage: 'Failed to load timezones!' });
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
            const msg = formatMessage({ defaultMessage: 'Invalid state, cannot move widget without combined scene!' });
            toast.error(msg);
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
                kind: targetWidgetState.kind,
                position: targetWidgetState.position,
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
        this.setState({ openDialogKind: 'scene-select', addPosition: position }, this.#loadRecentRemoteWidgets);
    };
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

        const position = widget.position;
        if (!position) {
            toast.error(formatMessage({ defaultMessage: 'Cannot continute editing, widget has no position!' }));
            return;
        }

        const wkind = widget.kind;
        switch (wkind?.value?.case) {
            case undefined:
                break;

            case 'clock': {
                this.setState(s => ({
                    openDialogKind: 'clock',
                    dialogStates: {
                        ...s.dialogStates,
                        clock: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackClockWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;
            }

            case 'tickerBtc': {
                this.setState(s => ({
                    openDialogKind: 'ticker',
                    dialogStates: {
                        ...s.dialogStates,
                        ticker: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackTicketWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;
            }

            case 'blockHeight': {
                this.setState(s => ({
                    openDialogKind: 'blockHeight',
                    dialogStates: {
                        ...s.dialogStates,
                        blockHeight: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackBlockHeightWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;
            }

            case 'blockchainData': {
                this.setState(s => ({
                    openDialogKind: 'blockchainData',
                    dialogStates: {
                        ...s.dialogStates,
                        blockchainData: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackBlockchainDataWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;
            }

            case 'braiinsPool': {
                this.setState(s => ({
                    openDialogKind: 'braiinsPool',
                    dialogStates: {
                        ...s.dialogStates,
                        braiinsPool: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackBraiinsPoolWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;
            }

            case 'remoteImage':
                this.setState(s => ({
                    openDialogKind: 'remoteImage',
                    dialogStates: {
                        ...s.dialogStates,
                        remoteImage: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackRemoteImageWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;

            case 'halvingCountdown':
                this.setState(s => ({
                    openDialogKind: 'halvingCountdown',
                    dialogStates: {
                        ...s.dialogStates,
                        halvingCountdown: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackHalvingCountdownWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;

            case 'remoteWidget':
                this.setState(s => ({
                    openDialogKind: 'remoteWidget',
                    dialogStates: {
                        ...s.dialogStates,
                        remoteWidget: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackRemoteWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;

            case 'countdown':
                this.setState(s => ({
                    openDialogKind: 'countdown',
                    dialogStates: {
                        ...s.dialogStates,
                        countdown: {
                            isEdit: true,
                            widgetID: id,
                            position,
                            data: {
                                errors: null,
                                values: Comp.unpackCountdownWidgetKind(wkind, widget.size),
                            },
                        },
                    },
                }));
                break;

            default:
                assertUndefined(wkind?.value, 'Unknown widget kind!');
        }
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

    #goBack = (): void => {
        this.props.navigate(URLS.pages.display.list);
    };
    #handleWidgetAdd = async (kind: Comp.SceneKind): Promise<void> => {
        const { sceneId } = this.props;
        const { formatMessage } = this.props.intl;
        const { openDialogKind, addPosition, remoteWidgetUrl } = this.state;

        if (!addPosition) {
            toast.error(formatMessage({ defaultMessage: "Can't add widget without position, aborting!" }));
            return;
        }
        if (openDialogKind !== 'scene-select') {
            toast.error(formatMessage({ defaultMessage: "Can't add widget without open dialog!" }));
            return;
        }
        const size = pb.WidgetSize.SMALL;

        // Here we have to call the widget RPC just with widget kind, size and position.
        // No attributes are read by backend at this point.
        let $openDialogKind: keyof DialogStates;
        let $widgetKind: pb.WidgetKind['value'];
        switch (kind) {
            case 'clock':
                $openDialogKind = 'clock';
                $widgetKind = { case: 'clock', value: pb.create(pb.ClockWidgetSchema) };
                break;

            case 'tickerBtc':
                $openDialogKind = 'ticker';
                $widgetKind = { case: 'tickerBtc', value: pb.create(pb.TickerBtcWidgetSchema) };
                break;

            case 'blockHeight':
                $openDialogKind = 'blockHeight';
                $widgetKind = { case: 'blockHeight', value: pb.create(pb.BlockHeightWidgetSchema) };
                break;

            case 'blockchainData':
                $openDialogKind = 'blockchainData';
                $widgetKind = { case: 'blockchainData', value: pb.create(pb.BlockchainDataWidgetSchema) };
                break;

            case 'braiinsPool':
                $openDialogKind = 'braiinsPool';
                $widgetKind = { case: 'braiinsPool', value: pb.create(pb.BraiinsPoolWidgetSchema) };
                break;

            case 'remoteImage':
                $openDialogKind = 'remoteImage';
                $widgetKind = { case: 'remoteImage', value: pb.create(pb.RemoteImageWidgetSchema) };
                break;

            case 'halvingCountdown':
                $openDialogKind = 'halvingCountdown';
                $widgetKind = { case: 'halvingCountdown', value: pb.create(pb.HalvingCountdownWidgetSchema) };
                break;

            case 'remoteWidget':
                $openDialogKind = 'remoteWidget';
                $widgetKind = {
                    case: 'remoteWidget',
                    // At the time of writing, the remote widget is the only one
                    // that actually reads these values upon creation, so we
                    // need to provide them and read potential errors.
                    value: pb.create(pb.RemoteWidgetSchema, { widgetUrl: remoteWidgetUrl.value }),
                };
                break;

            case 'countdown':
                $openDialogKind = 'countdown';
                $widgetKind = { case: 'countdown', value: pb.create(pb.CountdownWidgetSchema) };
                break;

            default:
                assertUnreachable(kind, 'Unknown widget kind!');
        }

        // We are default to `small` size because that is the most un-problematic size
        // and user can change it right away if there is enough space.
        //
        // This can fail with addition of remote widget
        // because it validates the provided URL.
        let widgetID: pb.Widget['id'];
        try {
            const res = await pb.rpc.scenes.addWidget(
                pb.create(pb.AddWidgetRequestSchema, {
                    sceneId,
                    size,
                    position: addPosition,
                    kind: { value: $widgetKind },
                }),
            );
            widgetID = res.value;
        } catch ($) {
            const e = pb.parseFormErrors<pb.AddFullscreenSceneRequest>($);
            const wke = e.fields.widgetKind as Maybe<Rec>;

            // We either manage to parse out errors for the remote widget URL…
            if (isPlainObject(wke) && 'remoteWidget' in wke && Array.isArray(wke.remoteWidget)) {
                const errors = wke.remoteWidget as string[];
                this.setState(s => ({ remoteWidgetUrl: { ...s.remoteWidgetUrl, errors } }));
            }

            // …or just notify generically that something has failed
            else {
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to add display widget!' });
                toast.error(msg);
            }

            return;
        }

        // Now that the widget has been added, we need to re-load the scene
        // to get the new widget data (as we only submit the position and type).
        // This will make our state consistent with the server.
        const scene = await this.#loadScene();
        const dialogStates = getInitialDialogStates();
        if (scene?.kind?.case === 'combined') {
            const widgetKind = scene.kind.value.widgets.find(x => x.id === widgetID)?.kind;
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

        dialogStates[$openDialogKind].isEdit = false;
        dialogStates[$openDialogKind].position = addPosition;
        dialogStates[$openDialogKind].widgetID = widgetID;

        this.setState({
            dialogStates,
            openDialogKind: $openDialogKind,
            remoteWidgetUrl: { value: '', errors: null },
        });
    };
    #handleWidgetAddRemote = async (): Promise<void> => {
        this.#handleWidgetAdd('remoteWidget');
    };
    #sceneRemoteWidgetUrlChange = (url: string): void => {
        this.setState({ remoteWidgetUrl: { value: url, errors: null } });
    };
    #openDialogCancel = (): void => this.setState({ openDialogKind: null, dialogStates: getInitialDialogStates() });

    #getChangeHandler = <
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
            }, this.#submitDebounced);
        };
    };
    #getValue = <
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
    #getError = <
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
    #getField = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['data']['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        return {
            value: this.#getValue(widgetKind, fieldKey),
            error: this.#getError(widgetKind, fieldKey),
            onChange: this.#getChangeHandler(widgetKind, fieldKey),
            disabled: false,
        };
    };

    #submit = async (): Promise<void> => {
        const { sceneId } = this.props;
        const { formatMessage } = this.props.intl;
        const { scene, openDialogKind, dialogStates } = this.state;

        if (scene?.kind.case !== 'combined') {
            toast.error(formatMessage({ defaultMessage: 'Cannot submit without combined scene!' }));
            return;
        }

        const widgets = scene.kind.value.widgets;
        if (!widgets) {
            toast.error(formatMessage({ defaultMessage: 'Scene edit: no widget data, aborting!' }));
            return;
        }

        let id: pb.Widget['id'];
        let size: pb.WidgetSize = pb.WidgetSize.SMALL;
        let kind: pb.WidgetKind;
        let position: pb.WidgetPosition;
        switch (openDialogKind) {
            case null:
            case 'scene-select':
                toast.error(formatMessage({ defaultMessage: 'Invalid state, cannot submit without open dialog!' }));
                return;

            case 'clock':
                id = dialogStates.clock.widgetID;
                size = dialogStates.clock.data.values.widgetSize ?? size;
                position = dialogStates.clock.position;
                kind = Comp.createClockWidgetKind(dialogStates.clock.data.values);
                break;

            case 'ticker':
                id = dialogStates.ticker.widgetID;
                size = dialogStates.ticker.data.values.widgetSize ?? size;
                position = dialogStates.ticker.position;
                kind = Comp.createTickerWidgetKind(dialogStates.ticker.data.values);
                break;

            case 'blockHeight':
                id = dialogStates.blockHeight.widgetID;
                size = dialogStates.blockHeight.data.values.widgetSize ?? size;
                position = dialogStates.blockHeight.position;
                kind = Comp.createBlockHeightWidgetKind(dialogStates.blockHeight.data.values);
                break;

            case 'blockchainData':
                id = dialogStates.blockchainData.widgetID;
                size = dialogStates.blockchainData.data.values.widgetSize ?? size;
                position = dialogStates.blockchainData.position;
                kind = Comp.createBlockchainDataWidgetKind(dialogStates.blockchainData.data.values);
                break;

            case 'braiinsPool':
                id = dialogStates.braiinsPool.widgetID;
                size = dialogStates.braiinsPool.data.values.widgetSize ?? size;
                position = dialogStates.braiinsPool.position;
                kind = Comp.createBraiinsPoolWidgetKind(dialogStates.braiinsPool.data.values);
                break;

            case 'remoteImage':
                id = dialogStates.remoteImage.widgetID;
                size = dialogStates.remoteImage.data.values.widgetSize ?? size;
                position = dialogStates.remoteImage.position;
                kind = Comp.createRemoteImageWidgetKind(dialogStates.remoteImage.data.values);
                break;

            case 'halvingCountdown':
                id = dialogStates.halvingCountdown.widgetID;
                size = dialogStates.halvingCountdown.data.values.widgetSize ?? size;
                position = dialogStates.halvingCountdown.position;
                kind = Comp.createHalvingCountdownWidgetKind(dialogStates.halvingCountdown.data.values);
                break;

            case 'remoteWidget':
                id = dialogStates.remoteWidget.widgetID;
                size = dialogStates.remoteWidget.data.values.widgetSize ?? size;
                position = dialogStates.remoteWidget.position;
                kind = Comp.createRemoteWidgetKind(dialogStates.remoteWidget.data.values);
                break;

            case 'countdown':
                id = dialogStates.countdown.widgetID;
                size = dialogStates.countdown.data.values.widgetSize ?? size;
                position = dialogStates.countdown.position;
                kind = Comp.createCountdownWidgetKind(dialogStates.countdown.data.values);
                break;

            default:
                assertUnreachable(openDialogKind, 'Unknown open dialog kind!');
        }

        const canonicalInsertPosition = fn.getWidgetInsertionSlot(widgets, { id, size, position });
        if (!canonicalInsertPosition) {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, widget seems not to fit!' }));
            return;
        }

        const payload = pb.create(pb.UpdateWidgetRequestSchema, {
            id,
            kind,
            sceneId,
            size,
            position: canonicalInsertPosition,
        });
        try {
            await pb.rpc.scenes.updateWidget(payload);
            toast.success(formatMessage({ defaultMessage: 'Widget updated!' }));
        } catch ($) {
            const formErrors = pb.parseFormErrors($, ['sceneId', 'position', 'size', 'kind']);
            console.log('formErrors', formErrors);
            this.setState(s => ({
                dialogStates: {
                    ...s.dialogStates,
                    [openDialogKind]: {
                        ...s.dialogStates[openDialogKind],
                        errors: formErrors,
                    },
                },
            }));
        }

        return this.#loadSceneDebounced();
    };
    #submitDebounced = debounce(this.#submit, 300);

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
        const { intl } = this.props;
        const {
            scene,
            accounts,
            timezones,
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
        } = this.state;

        const widgets: pb.Widget[] = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];

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
                    onSelection={this.#handleWidgetAdd}
                    remoteWidgetUrl={{
                        value: remoteWidgetUrl.value,
                        error: pb.renderFieldErrorsAsList(remoteWidgetUrl.errors),
                        disabled: false,
                        onChange: this.#sceneRemoteWidgetUrlChange,
                        onSubmit: this.#handleWidgetAddRemote,
                    }}
                    remoteWidgetRecents={recentRemoteWidgets}
                />

                <Comp.FormWidgetClock
                    isOpen={openDialogKind === 'clock'}
                    isEdit={clock.isEdit}
                    onClose={this.#openDialogCancel}
                    error={openDialogKind === 'clock' ? pb.renderFieldErrorsAsList(clock.data?.errors?.global) : null}
                    // Fields
                    widgetSize={{
                        ...this.#getField('clock', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: clock.widgetID,
                            position: clock.position,
                        }),
                    }}
                    clockStyle={this.#getField('clock', 'clockStyle')}
                    fontStyle={this.#getField('clock', 'fontStyle')}
                    showDate={this.#getField('clock', 'showDate')}
                    showSeconds={this.#getField('clock', 'showSeconds')}
                    showTimezone={this.#getField('clock', 'showTimezone')}
                    timezone={{ ...this.#getField('clock', 'timezone'), options: timezones }}

                    // showWeather={this.#getFormFieldStruct('clock', 'showWeather')}
                    // weatherLocation={this.#getFormFieldStruct('clock', 'weatherLocation')}
                />
                <Comp.FormWidgetTicker
                    isOpen={openDialogKind === 'ticker'}
                    isEdit={ticker.isEdit}
                    onClose={this.#openDialogCancel}
                    error={openDialogKind === 'ticker' ? pb.renderFieldErrorsAsList(ticker.data?.errors?.global) : null}
                    // Fields
                    widgetSize={{
                        ...this.#getField('ticker', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: ticker.widgetID,
                            position: ticker.position,
                        }),
                    }}
                    timeFrame={this.#getField('ticker', 'timeFrame')}
                />
                <Comp.FormWidgetBlockHeight
                    isOpen={openDialogKind === 'blockHeight'}
                    isEdit={blockHeight.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'blockHeight'
                            ? pb.renderFieldErrorsAsList(blockHeight.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('blockHeight', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: blockHeight.widgetID,
                            position: blockHeight.position,
                        }),
                    }}
                    fontStyle={this.#getField('blockHeight', 'fontStyle')}
                    showDate={this.#getField('blockHeight', 'showDate')}
                />
                <Comp.FormWidgetBlockchainData
                    isOpen={openDialogKind === 'blockchainData'}
                    isEdit={blockchainData.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'blockchainData'
                            ? pb.renderFieldErrorsAsList(blockchainData.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('blockchainData', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: blockchainData.widgetID,
                            position: blockchainData.position,
                        }),
                    }}
                />
                <Comp.FormWidgetCountdown
                    isOpen={openDialogKind === 'countdown'}
                    isEdit={countdown.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'countdown'
                            ? pb.renderFieldErrorsAsList(countdown.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('countdown', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: countdown.widgetID,
                            position: countdown.position,
                        }),
                    }}
                    label={this.#getField('countdown', 'label')}
                    targetDate={this.#getField('countdown', 'targetDate')}
                    targetTime={this.#getField('countdown', 'targetTime')}
                    backgroundColor={this.#getField('countdown', 'backgroundColor')}
                    fontStyle={this.#getField('countdown', 'fontStyle')}
                    ledEnabled={this.#getField('countdown', 'ledEnabled')}
                    ledEffect={this.#getField('countdown', 'ledEffect')}
                    ledColorR={this.#getField('countdown', 'ledColorR')}
                    ledColorG={this.#getField('countdown', 'ledColorG')}
                    ledColorB={this.#getField('countdown', 'ledColorB')}
                    soundEnabled={this.#getField('countdown', 'soundEnabled')}
                    soundId={this.#getField('countdown', 'soundId')}
                    soundVolume={this.#getField('countdown', 'soundVolume')}
                    soundOptions={this.state.sounds}
                />
                <Comp.FormWidgetBraiinsPool
                    isOpen={openDialogKind === 'braiinsPool'}
                    isEdit={braiinsPool.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'braiinsPool'
                            ? pb.renderFieldErrorsAsList(braiinsPool.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('braiinsPool', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: braiinsPool.widgetID,
                            position: braiinsPool.position,
                        }),
                    }}
                    accountId={{
                        ...this.#getField('braiinsPool', 'accountId'),
                        options: accounts,
                    }}
                    sceneStyle={this.#getField('braiinsPool', 'sceneStyle')}
                    timeFrame={this.#getField('braiinsPool', 'timeFrame')}
                />
                <Comp.FormWidgetRemoteImage
                    isOpen={openDialogKind === 'remoteImage'}
                    isEdit={remoteImage.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'remoteImage'
                            ? pb.renderFieldErrorsAsList(remoteImage.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('remoteImage', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: remoteImage.widgetID,
                            position: remoteImage.position,
                        }),
                    }}
                    url={this.#getField('remoteImage', 'url')}
                    refreshDurationSec={this.#getField('remoteImage', 'refreshDurationSec')}
                />
                <Comp.FormWidgetHalvingCountdown
                    isOpen={openDialogKind === 'halvingCountdown'}
                    isEdit={halvingCountdown.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'halvingCountdown'
                            ? pb.renderFieldErrorsAsList(halvingCountdown.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('halvingCountdown', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: halvingCountdown.widgetID,
                            position: halvingCountdown.position,
                        }),
                    }}
                />
                <Comp.FormWidgetRemoteWidget
                    isOpen={openDialogKind === 'remoteWidget'}
                    isEdit={remoteWidget.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialogKind === 'remoteWidget'
                            ? pb.renderFieldErrorsAsList(remoteWidget.data?.errors?.global)
                            : null
                    }
                    // Fields
                    widgetSize={{
                        ...this.#getField('remoteWidget', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: remoteWidget.widgetID,
                            position: remoteWidget.position,
                        }),
                    }}
                    url={this.#getField('remoteWidget', 'url')}
                    name={this.#getField('remoteWidget', 'name')}
                    params={this.#getField('remoteWidget', 'params')}
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
