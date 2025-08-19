import { Component } from 'react';
import { debounce, cloneDeep } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { useParams, useNavigate, type NavigateFunction } from 'react-router';

// Libs
import * as fn from './fn';
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
import {
    CombinedSceneView,
    type CombinedSceneViewProps,
    FormSceneSelect,
    FormWidgetClock,
    type FormWidgetClockProps,
    type SceneKind,
} from './components';

// Styles
import css from './DisplayCombined.scss';

type FormStateClock = FormPropsToLocalState<FormWidgetClockProps>;

interface Props {
    navigate: NavigateFunction;
    intl: IntlShape;
    sceneId: string;
}

interface State {
    isLoading: boolean;

    timezones: pb.Timezone[];
    scene: null | pb.Scene;

    openDialog:
        | null
        | {
              key: 'scene-select';
              position: pb.WidgetPosition;
          }
        // Can be both edit & create dialogs
        | {
              key: 'scene-config-clock';
              data: null | FormStateClock;
              isEdit: boolean;
              widgetID: string;
              position: pb.WidgetPosition;
          };
}
const getInitialState = (): State => ({
    isLoading: false,

    timezones: [],
    scene: null,

    openDialog: null,
});

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

    #handleMove: CombinedSceneViewProps['onWidgetMove'] = async (
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
        this.setState({
            openDialog: {
                key: 'scene-select',
                position,
            },
        });
    };
    #handleEdit: CombinedSceneViewProps['onWidgetEdit'] = (id: string): void => {
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

        if (!widget.position) {
            notify('error', 'Cannot continute editing, widget has no position!');
            return;
        }

        switch (widget.kind?.value.case) {
            case 'clock': {
                const w = widget.kind.value.value;
                this.setState({
                    openDialog: {
                        key: 'scene-config-clock',
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
                        position: widget.position,
                    },
                });
            }
        }
    };
    #handleRemove: CombinedSceneViewProps['onWidgetRemove'] = async (widgetId: string): Promise<void> => {
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
    #handleWidgetAdd = async (kind: SceneKind): Promise<void> => {
        const { notify } = this.context;
        const { sceneId } = this.props;
        const { openDialog } = this.state;

        if (openDialog?.key !== 'scene-select') {
            notify('error', 'Invalid state, cannot add widget without open dialog!');
            return;
        }
        const { position } = openDialog;

        if (kind === 'combined') {
            notify('error', 'Cannot add a combined scene to a combined scene, aborting!');
            return;
        }

        switch (kind) {
            case 'clock': {
                const response = await pb.rpc.scenes.addWidget(
                    pb.create(pb.AddWidgetRequestSchema, {
                        sceneId,
                        position,
                        size: pb.WidgetSize.SMALL,
                        kind: {
                            value: {
                                case: kind,
                                value: pb.create(pb.ClockWidgetSchema),
                            },
                        },
                    }),
                );

                await this.#loadScene();

                this.setState({
                    openDialog: {
                        key: 'scene-config-clock',
                        data: {
                            values: {
                                widgetSize: pb.WidgetSize.SMALL,
                                clockStyle: pb.ClockWidget_ClockStyle.ANALOG_ROUND,
                                fontStyle: pb.FontStyle.LIGHT,
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: undefined,
                            },
                            errors: null,
                        },
                        isEdit: false,
                        widgetID: response.value,
                        position,
                    },
                });
                break;
            }

            default:
                assertUnreachable(kind, 'Unknown widget kind!');
        }
    };
    #openDialogCancel = (): void => this.setState({ openDialog: null });

    #clockGetChangeHandler = <Key extends keyof FormStateClock['values']>(key: Key) => {
        return (value: FormStateClock['values'][Key]) => {
            this.setState(s => {
                const d = s.openDialog;
                if (d?.key !== 'scene-config-clock') {
                    this.context.notify('error', 'Invalid state, cannot change clock settings without open dialog!');
                    return s;
                }

                return {
                    ...s,
                    openDialog: {
                        ...d,
                        data: {
                            errors: null,
                            values: {
                                ...d.data?.values,
                                [key]: value,
                            },
                        },
                    },
                };
            }, this.#submit);
        };
    };
    #clockGetFieldValue = <Key extends keyof FormStateClock['values']>(key: Key) => {
        const x = this.state.openDialog;
        if (x?.key !== 'scene-config-clock') return null;
        return x.data?.values?.[key] ?? null;
    };
    #clockGetFieldError = <Key extends keyof FormStateClock['values']>(key: Key): null | string => {
        const x = this.state.openDialog;
        if (x?.key !== 'scene-config-clock') return null;
        return pb.renderFieldErrorsAsList(x.data?.errors?.fields?.[key]);
    };

    #submit = async (): Promise<void> => {
        const { notify } = this.context;
        const { scene } = this.state;
        const { sceneId } = this.props;

        if (scene?.kind.case !== 'combined') {
            notify('error', 'Invalid state, cannot submit without combined scene!');
            return;
        }

        const widgets = scene.kind.value.widgets;
        if (!widgets) {
            notify('error', 'Scene edit: no widget data, aborting!');
            return;
        }

        const x = this.state.openDialog;
        if (x?.key !== 'scene-config-clock') {
            notify('error', 'Invalid state, cannot submit without open dialog!');
            return;
        }

        const { data, widgetID, position } = x;
        if (!data?.values) {
            notify('error', 'Scene edit: no data, aborting!');
            return;
        }

        const canonicalInsertPosition = fn.getWidgetInsertionSlot(widgets, {
            id: widgetID,
            position,
            size: data.values.widgetSize ?? pb.WidgetSize.SMALL,
        });
        if (!canonicalInsertPosition) {
            notify('error', 'Invalid state, widget seems not to fit!');
            return;
        }

        const payload = pb.create(pb.UpdateWidgetRequestSchema, {
            id: widgetID,
            sceneId: sceneId,
            kind: pb.create(pb.WidgetKindSchema, {
                value: {
                    case: 'clock',
                    value: pb.create(pb.ClockWidgetSchema, {
                        clockStyle: data.values.clockStyle,
                        numbersFontStyle: data.values.fontStyle,
                        showDate: data.values.showDate,
                        showSeconds: data.values.showSeconds,

                        showTimezone: data.values.showTimezone,
                        timezone: data.values.timezone,
                    }),
                },
            }),
            size: data.values.widgetSize,
            position: canonicalInsertPosition,
        });
        try {
            await pb.rpc.scenes.updateWidget(payload);
            notify('success', 'Widget updated!', { id: 'combined-scene-widget-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            const formErrors = pb.parseFormErrors($, ['sceneId', 'position', 'size', 'kind']);
            this.setState(s => {
                const openDialog = cloneDeep(s.openDialog);
                if (openDialog?.key !== 'scene-config-clock') return s;

                openDialog.data = {
                    values: data.values,
                    errors: formErrors,
                };
                return { ...s, openDialog };
            });
        }

        this.#loadSceneDebounced();
    };

    render() {
        const { intl } = this.props;
        const { scene, timezones, openDialog } = this.state;

        const clockFormData = openDialog?.key === 'scene-config-clock' ? openDialog : null;
        const widgets: pb.Widget[] = scene?.kind.case === 'combined' ? scene.kind.value.widgets : [];

        return (
            <div className={css.root}>
                <Helmet title={this.#txt.title} />
                <header className={css.header}>
                    <div className={css.headerLeft}>
                        <Button
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

                <CombinedSceneView
                    widgets={widgets}
                    onWidgetMove={this.#handleMove}
                    onWidgetAdd={this.#handleAdd}
                    onWidgetEdit={this.#handleEdit}
                    onWidgetRemove={this.#handleRemove}
                />

                <FormSceneSelect
                    variant="widget"
                    isOpen={openDialog?.key === 'scene-select'}
                    onClose={this.#openDialogCancel}
                    onSelection={this.#handleWidgetAdd}
                />

                <FormWidgetClock
                    isOpen={!!clockFormData}
                    isEdit={!!clockFormData?.isEdit}
                    onClose={this.#openDialogCancel}
                    error={
                        openDialog?.key === 'scene-config-clock'
                            ? pb.renderFieldErrorsAsList(openDialog.data?.errors?.global)
                            : null
                    }
                    widgetSize={{
                        value: this.#clockGetFieldValue('widgetSize') ?? pb.WidgetSize.SMALL,
                        error: this.#clockGetFieldError('widgetSize'),
                        onChange: this.#clockGetChangeHandler('widgetSize'),
                        options: clockFormData
                            ? fn.getValidWidgetSizes(widgets, {
                                  id: clockFormData.widgetID,
                                  position: clockFormData.position,
                              })
                            : [],
                    }}
                    clockStyle={{
                        value: this.#clockGetFieldValue('clockStyle'),
                        error: this.#clockGetFieldError('clockStyle'),
                        onChange: this.#clockGetChangeHandler('clockStyle'),
                        disabled: false,
                    }}
                    fontStyle={{
                        value: this.#clockGetFieldValue('fontStyle'),
                        error: this.#clockGetFieldError('fontStyle'),
                        onChange: this.#clockGetChangeHandler('fontStyle'),
                        disabled: false,
                    }}
                    showDate={{
                        value: this.#clockGetFieldValue('showDate'),
                        error: this.#clockGetFieldError('showDate'),
                        onChange: this.#clockGetChangeHandler('showDate'),
                        disabled: false,
                    }}
                    showSeconds={{
                        value: this.#clockGetFieldValue('showSeconds'),
                        error: this.#clockGetFieldError('showSeconds'),
                        onChange: this.#clockGetChangeHandler('showSeconds'),
                        disabled: false,
                    }}
                    showTimezone={{
                        value: this.#clockGetFieldValue('showTimezone'),
                        error: this.#clockGetFieldError('showTimezone'),
                        onChange: this.#clockGetChangeHandler('showTimezone'),
                        disabled: false,
                    }}
                    timezone={{
                        value: this.#clockGetFieldValue('timezone'),
                        error: this.#clockGetFieldError('timezone'),
                        onChange: this.#clockGetChangeHandler('timezone'),
                        options: timezones,
                        disabled: false,
                    }}

                    // showWeather={{
                    //     value: this.#clockGetFieldValue('showWeather'),
                    //     error: this.#clockGetFieldError('showWeather'),
                    //     onChange: this.#clockGetChangeHandler('showWeather'),
                    //     disabled: false,
                    // }}
                    // weatherLocation={{
                    //     value: this.#clockGetFieldValue('weatherLocation'),
                    //     error: this.#clockGetFieldError('weatherLocation'),
                    //     onChange: this.#clockGetChangeHandler('weatherLocation'),
                    //     disabled: false,
                    // }}
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
