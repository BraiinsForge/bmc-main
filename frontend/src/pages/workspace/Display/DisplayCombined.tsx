import { Component } from 'react';
import { debounce, cloneDeep } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { useParams, useNavigate, type NavigateFunction } from 'react-router';

// Libs
import * as fn from './fn';
import { getID } from './const.ts';
import { setState } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts.ts';
import type { FormPropsToLocalState } from '@/lib/form';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Button } from '@/components';
import { ChevronLeft as IconChevronLeft } from '@carbon/react/icons';
import * as Comp from './components';

// Styles
import css from './DisplayCombined.scss';

type FormStateClock = FormPropsToLocalState<Comp.FormWidgetClockProps>;
type FormStateTicker = FormPropsToLocalState<Comp.FormWidgetTickerProps>;
type FormStateBlockHeight = FormPropsToLocalState<Comp.FormWidgetBlockHeightProps>;

// Can be both edit & create dialogs
type FormDialogState<Data> = {
    data: Data;
    isEdit: boolean;
    widgetID: string;
    position: pb.WidgetPosition;
};
type DialogStates = {
    clock: FormDialogState<FormStateClock>;
    ticker: FormDialogState<FormStateTicker>;
    blockHeight: FormDialogState<FormStateBlockHeight>;
};
function getInitialDialogStates(): DialogStates {
    const position = pb.create(pb.WidgetPositionSchema);
    const sharedBase = { isEdit: false, widgetID: '', position } as const;
    const widgetSize = pb.WidgetSize.SMALL;
    const fontStyle = pb.FontStyle.LIGHT;

    return {
        clock: {
            ...sharedBase,
            data: {
                errors: null,
                values: {
                    widgetSize,
                    fontStyle,
                    clockStyle: pb.ClockWidget_ClockStyle.ANALOG_ROUND,
                    showDate: true,
                    showSeconds: true,
                    showTimezone: true,
                    timezone: undefined,
                },
            },
        },
        ticker: {
            ...sharedBase,
            data: {
                errors: null,
                values: {
                    widgetSize,
                    timeFrame: pb.TickerBtcWidget_TimeFrame.DAY_1,
                },
            },
        },
        blockHeight: {
            ...sharedBase,
            data: {
                errors: null,
                values: {
                    showDate: true,
                    fontStyle,
                    widgetSize,
                },
            },
        },
    };
}

interface Props {
    navigate: NavigateFunction;
    intl: IntlShape;
    sceneId: string;
}

interface State {
    isLoading: boolean;

    timezones: pb.Timezone[];
    scene: null | pb.Scene;

    openDialogKind: null | 'scene-select' | keyof DialogStates;
    addPosition: null | pb.WidgetPosition;
    dialogStates: DialogStates;
}
const getInitialState = (): State => ({
    isLoading: false,

    timezones: [],
    scene: null,

    openDialogKind: null,
    addPosition: null,
    dialogStates: getInitialDialogStates(),
});

const $ = getID('combined').get;
class View extends Component<Props, State> {
    readonly state = getInitialState();
    static contextType = AppContext;
    declare context: AppContextType;

    componentDidMount() {
        this.#loadScene();
        this.#loadMetadata();
        this.#previewOpen();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    #txt = {
        title: this.props.intl.formatMessage({ defaultMessage: 'Edit Combined Scene' }),
    };

    private abortLoadMetadata = pb.abort.get();
    #loadMetadata = async (): Promise<void> => {
        const { notify } = this.context;
        const { intl } = this.props;

        try {
            const { signal } = this.abortLoadMetadata.replace();
            const { timezones } = await pb.rpc.sys.getTimezoneList({}, { signal });
            this.setState({ timezones });
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string =
                pb.collectAllErrorsAsFormattedList($) ??
                intl.formatMessage({ defaultMessage: 'Failed to load timezones!' });
            notify('error', msg);
        }
    };

    private abortLoadScene = pb.abort.get();
    #loadScene = async (): Promise<void> => {
        const { sceneId } = this.props;
        await setState(this, { isLoading: true });

        try {
            const { signal } = this.abortLoadScene.replace();
            const { scene } = await pb.rpc.scenes.getScene({ value: sceneId }, { signal });
            this.setState({ isLoading: false, scene: scene || null });
        } catch ($) {
            if (pb.abort.is($)) return;
        }
    };
    #loadSceneDebounced = debounce(this.#loadScene, 200);

    private abortPreview = pb.abort.get();
    #previewOpen = async (): Promise<void> => {
        const { sceneId, intl } = this.props;
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

    #handleMove: Comp.CombinedSceneViewProps['onWidgetMove'] = async (
        source: pb.Widget,
        target: pb.Widget,
    ): Promise<void> => {
        const { notify } = this.context;
        const { sceneId } = this.props;
        const scene = cloneDeep(this.state.scene);

        if (scene?.kind.case !== 'combined') {
            notify('error', 'Invalid state, cannot move widget without combined scene!');
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
            notify('error', 'Invalid state, widget seems not to fit!');
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
            const msg: string = pb.collectAllErrorsAsFormattedList($) ?? 'Failed to update widget!';
            notify('error', msg, { id: 'display-widget-move', timeoutSeconds: 2 });
        }

        this.#loadSceneDebounced();
    };
    #handleAdd = (position: pb.WidgetPosition): void => {
        this.setState({ openDialogKind: 'scene-select', addPosition: position });
    };
    #handleEdit: Comp.CombinedSceneViewProps['onWidgetEdit'] = (id: string): void => {
        const { notify } = this.context;

        const scene = this.state.scene;
        if (scene?.kind.case !== 'combined') {
            notify('error', 'Invalid state, cannot edit widget without combined scene!');
            return;
        }

        const widget = scene.kind.value.widgets.find(x => x.id === id);
        if (!widget) {
            notify('error', 'Invalid state, widget data not found!');
            return;
        }

        const position = widget.position;
        if (!position) {
            notify('error', 'Cannot continute editing, widget has no position!');
            return;
        }

        const value = widget.kind?.value;
        switch (value?.case) {
            case undefined:
                break;

            case 'clock': {
                const w = value.value;
                this.setState(s => ({
                    openDialogKind: 'clock',
                    dialogStates: {
                        ...s.dialogStates,
                        clock: {
                            data: {
                                values: {
                                    widgetSize: widget.size,

                                    clockStyle: w.clockStyle,
                                    fontStyle: w.numbersFontStyle,

                                    showDate: w.showDate,
                                    showSeconds: w.showSeconds,

                                    showTimezone: w.showTimezone,
                                    timezone: w.timezone,
                                },
                                errors: null,
                            },
                            isEdit: true,
                            widgetID: id,
                            position,
                        },
                    },
                }));
                break;
            }

            case 'tickerBtc': {
                const w = value.value;
                this.setState(s => ({
                    openDialogKind: 'ticker',
                    dialogStates: {
                        ...s.dialogStates,
                        ticker: {
                            data: {
                                errors: null,
                                values: {
                                    widgetSize: widget.size,
                                    timeFrame: w.timeFrame,
                                },
                            },
                            isEdit: true,
                            widgetID: id,
                            position,
                        },
                    },
                }));
                break;
            }

            case 'blockHeight': {
                const w = value.value;
                this.setState(s => ({
                    openDialogKind: 'blockHeight',
                    dialogStates: {
                        ...s.dialogStates,
                        blockHeight: {
                            data: {
                                errors: null,
                                values: {
                                    widgetSize: widget.size,
                                    showDate: w.showTimestamp,
                                    fontStyle: w.numbersFontStyle,
                                },
                            },
                            isEdit: true,
                            widgetID: id,
                            position,
                        },
                    },
                }));
                break;
            }

            default:
                assertUnreachable(value, 'Unknown widget kind!');
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
        const { notify } = this.context;
        const { sceneId } = this.props;
        const { openDialogKind, addPosition } = this.state;

        if (kind === 'combined') return notify('error', "Can't add a combined scene to a combined scene, aborting!");
        if (!addPosition) return notify('error', "Can't add widget without position, aborting!");
        if (openDialogKind !== 'scene-select') return notify('error', "Can't add widget without open dialog!");

        let $openDialogKind: keyof DialogStates;
        let $widgetKind: pb.WidgetKind['value'];
        const $size = pb.WidgetSize.SMALL;
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

            default:
                assertUnreachable(kind, 'Unknown widget kind!');
        }

        const response = await pb.rpc.scenes.addWidget(
            pb.create(pb.AddWidgetRequestSchema, {
                sceneId,
                size: $size,
                position: addPosition,
                kind: { value: $widgetKind },
            }),
        );

        await this.#loadScene();

        const dialogStates = getInitialDialogStates();
        this.setState({
            openDialogKind: $openDialogKind,
            dialogStates: {
                ...dialogStates,
                [$openDialogKind]: {
                    ...dialogStates[$openDialogKind],
                    isEdit: false,
                    position: addPosition,
                    widgetID: response.value,
                },
            },
        });
    };
    #openDialogCancel = (): void => this.setState({ openDialogKind: null, dialogStates: getInitialDialogStates() });

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
            }, this.#submit);
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

    #submit = async (): Promise<void> => {
        const { notify } = this.context;
        const { sceneId } = this.props;
        const { scene, openDialogKind, dialogStates } = this.state;

        if (scene?.kind.case !== 'combined')
            return notify('error', 'Invalid state, cannot submit without combined scene!');

        const widgets = scene.kind.value.widgets;
        if (!widgets) return notify('error', 'Scene edit: no widget data, aborting!');

        let id: pb.Widget['id'];
        let size: pb.WidgetSize = pb.WidgetSize.SMALL;
        let kind: pb.WidgetKind;
        let position: pb.WidgetPosition;
        switch (openDialogKind) {
            case null:
            case 'scene-select':
                return notify('error', 'Invalid state, cannot submit without open dialog!');

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

            default:
                assertUnreachable(openDialogKind, 'Unknown open dialog kind!');
        }

        const canonicalInsertPosition = fn.getWidgetInsertionSlot(widgets, { id, size, position });
        if (!canonicalInsertPosition) return notify('error', 'Invalid state, widget seems not to fit!');

        const payload = pb.create(pb.UpdateWidgetRequestSchema, {
            id,
            kind,
            sceneId,
            size,
            position: canonicalInsertPosition,
        });
        try {
            await pb.rpc.scenes.updateWidget(payload);
            notify('success', 'Widget updated!', { id: 'combined-scene-widget-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            const formErrors = pb.parseFormErrors($, ['sceneId', 'position', 'size', 'kind']);
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

        this.#loadSceneDebounced();
    };

    render() {
        const { intl } = this.props;
        const {
            scene,
            timezones,
            openDialogKind,
            dialogStates: { clock, ticker, blockHeight },
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

                <Comp.CombinedSceneView
                    widgets={widgets}
                    onWidgetMove={this.#handleMove}
                    onWidgetAdd={this.#handleAdd}
                    onWidgetEdit={this.#handleEdit}
                    onWidgetRemove={this.#handleRemove}
                />

                <Comp.FormSceneSelect
                    variant="widget"
                    isOpen={openDialogKind === 'scene-select'}
                    onClose={this.#openDialogCancel}
                    onSelection={this.#handleWidgetAdd}
                />

                <Comp.FormWidgetClock
                    isOpen={openDialogKind === 'clock'}
                    isEdit={clock.isEdit}
                    onClose={this.#openDialogCancel}
                    error={openDialogKind === 'clock' ? pb.renderFieldErrorsAsList(clock.data?.errors?.global) : null}
                    // Fields
                    widgetSize={{
                        ...this.#getFormFieldStruct('clock', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: clock.widgetID,
                            position: clock.position,
                        }),
                    }}
                    clockStyle={this.#getFormFieldStruct('clock', 'clockStyle')}
                    fontStyle={this.#getFormFieldStruct('clock', 'fontStyle')}
                    showDate={this.#getFormFieldStruct('clock', 'showDate')}
                    showSeconds={this.#getFormFieldStruct('clock', 'showSeconds')}
                    showTimezone={this.#getFormFieldStruct('clock', 'showTimezone')}
                    timezone={{ ...this.#getFormFieldStruct('clock', 'timezone'), options: timezones }}

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
                        ...this.#getFormFieldStruct('ticker', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: ticker.widgetID,
                            position: ticker.position,
                        }),
                    }}
                    timeFrame={this.#getFormFieldStruct('ticker', 'timeFrame')}
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
                        ...this.#getFormFieldStruct('blockHeight', 'widgetSize'),
                        options: fn.getValidWidgetSizes(widgets, {
                            id: blockHeight.widgetID,
                            position: blockHeight.position,
                        }),
                    }}
                    fontStyle={this.#getFormFieldStruct('blockHeight', 'fontStyle')}
                    showDate={this.#getFormFieldStruct('blockHeight', 'showDate')}
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
