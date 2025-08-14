import { Component, createRef, Fragment } from 'react';
import { debounce } from 'es-toolkit';
import { formatDuration } from 'date-fns';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { useNavigate, type NavigateFunction } from 'react-router';

// Libs
import { Sized, setState } from '@/lib/react';
import { listenDocumentEvent } from '@/lib/dom';
import { Form, getID, type iField, type FormPropsToLocalState } from '@/lib/form';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Button, Modal } from '@/components';
import { Dropdown, Popover, PopoverContent, Toggle } from '@carbon/react';
import {
    Add as IconAdd,
    CarouselHorizontal as IconCycle,
    ChevronDown as IconChevronDown,
    ChevronUp as IconChevronUp,
} from '@carbon/react/icons';
import {
    FormSceneSelect,
    SceneOverviewList,
    FormWidgetClock,
    type FormWidgetClockProps,
    type SceneKind,
} from './components';

// Styles
import css from './DisplayList.scss';

const $ = getID('display').get;
type FormStateClock = FormPropsToLocalState<FormWidgetClockProps>;

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
        effect: pb.SceneCycleEffect;
    };

    openDialog:
        | null
        | { key: 'scene-select' }
        // Can be both edit & create dialogs
        | {
              key: 'scene-config-clock';
              data: null | FormStateClock;
              isEdit: boolean;
              sceneID: string;
          };
}
const getInitialState = (): State => ({
    isLoading: false,

    timezones: [],
    scenes: [],

    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 30,
        effect: pb.SceneCycleEffect.Slide,
    },
    openDialog: null,
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

        this.#cycleOpenToggle();
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
        };
    }

    #openDialogSceneSelect = (): void => this.setState({ openDialog: { key: 'scene-select' } });
    #sceneAddSelectedKind = async (kind: SceneKind): Promise<void> => {
        const { navigate } = this.props;

        switch (kind) {
            case 'combined': {
                const response = await pb.rpc.scenes.addCombinedScene({});
                navigate(URLS.pages.display.combined.getHref(response.value), { replace: false });
                break;
            }

            // Full-screen widgets
            default: {
                const response = await pb.rpc.scenes.addFullscreenScene(
                    pb.create(pb.AddFullscreenSceneRequestSchema, {
                        widgetKind: {
                            value: {
                                case: kind,
                                // Only the discriminant is important and read,
                                // so we'll send an emtpy valid object.
                                value: pb.create(pb.ClockWidgetSchema),
                            },
                        },
                    }),
                );

                await this.#loadScenes();

                this.setState({
                    openDialog: {
                        key: 'scene-config-clock',
                        data: null,
                        isEdit: false,
                        sceneID: response.value,
                    },
                });
                break;
            }
        }
    };
    #openDialogCancel = (): void => {
        this.abortPreview.abort();
        this.setState({ openDialog: null });
    };

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
            }, this.#sceneFullscreenWidgetSubmit);
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

        const x = this.state.openDialog;
        if (x?.key !== 'scene-config-clock') {
            notify('error', 'Invalid state, cannot submit without open dialog!');
            return;
        }

        const scene = this.#getScene(x.sceneID);
        if (!scene) {
            notify('error', 'Scene edit: cannot find the scene data!');
            return;
        }
        if (scene.kind.case !== 'fullscreen') {
            notify('error', 'Scene edit: not a fullscreen widget, aborting!');
            return;
        }

        const widget = scene.kind.value.widget;
        if (!widget) {
            notify('error', 'Scene edit: no widget data, aborting!');
            return;
        }

        const data = x.data?.values;
        if (!data) {
            notify('error', 'Scene edit: no data, aborting!');
            return;
        }

        const payload = pb.create(pb.UpdateWidgetRequestSchema, {
            id: widget.id,
            sceneId: scene.id,
            kind: pb.create(pb.WidgetKindSchema, {
                value: {
                    case: 'clock',
                    value: pb.create(pb.ClockWidgetSchema, {
                        clockStyle: data.clockStyle,
                        numbersFontStyle: data.fontStyle,
                        showDate: data.showDate,
                        showSeconds: data.showSeconds,

                        showTimezone: data.showTimezone,
                        timezone: data.timezone,
                    }),
                },
            }),
            // These are given for a full-screen widget
            size: pb.WidgetSize.FULL,
            position: { row: 0, col: 0 },
        });
        try {
            await pb.rpc.scenes.updateWidget(payload);
            notify('success', 'Widget updated!', { id: 'widget-updated', timeoutSeconds: 1.5 });
        } catch ($) {
            const msg = pb.collectAllErrorsAsFormattedList($) ?? 'Failed to update widget!';
            notify('error', msg);
        }

        this.#loadScenesDebounced();
    };
    #sceneAddRender = (): ReactElement => {
        const { openDialog, timezones } = this.state;
        const cancel = this.#openDialogCancel;

        return (
            <Fragment>
                <FormSceneSelect
                    variant="scene"
                    isOpen={openDialog?.key === 'scene-select'}
                    onClose={cancel}
                    onSelection={this.#sceneAddSelectedKind}
                />

                <FormWidgetClock
                    isOpen={openDialog?.key === 'scene-config-clock'}
                    isEdit={openDialog?.key === 'scene-config-clock' && openDialog.isEdit}
                    onClose={cancel}
                    // No size selector for the fullscreen widgets we operate with here
                    widgetSize={null}
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
            // Optimistic update first
            this.setState(s => ({
                scenes: s.scenes.map(x => (x.id === id ? { ...x, cycleDurationSec: Number.parseInt(value, 10) } : x)),
            }));

            pb.rpc.scenes.updateScene(
                {
                    id,
                    enabled: this.#getScene(id)?.enabled ?? true,
                    cycleDurationSec: Number.parseInt(value, 10),
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
                switch (kind?.value.widget?.kind?.value.case) {
                    case 'clock': {
                        const v = kind.value.widget.kind.value.value;
                        this.setState(
                            {
                                openDialog: {
                                    key: 'scene-config-clock',
                                    data: {
                                        values: {
                                            clockStyle: v.clockStyle,
                                            fontStyle: v.numbersFontStyle,

                                            showDate: v.showDate,
                                            showSeconds: v.showSeconds,
                                            showTimezone: v.showTimezone,

                                            timezone: v.timezone,
                                        },
                                        errors: null,
                                    },
                                    isEdit: true,
                                    sceneID: id,
                                },
                            },
                            () => this.#previewOpen(id),
                        );
                    }
                }
                break;
            }
        }
    };

    //
    // /Scene list handlers
    //

    // Cycle Settings
    #cycleOpenToggle = (): void => {
        this.setState(s => ({ cycle: { ...s.cycle, isOpen: !s.cycle.isOpen } }));
    };
    #cycleActiveToggle = (): void => {
        this.setState(s => ({ cycle: { ...s.cycle, isActive: !s.cycle.isActive } }));
    };
    #cycleDurationChange = (value: number): void => {
        this.setState(s => ({ cycle: { ...s.cycle, defaultDurationSeconds: value } }));
    };
    #cycleEffectChange = (value: pb.SceneCycleEffect): void => {
        this.setState(s => ({ cycle: { ...s.cycle, effect: value } }));
    };

    #headerRender = (): ReactElement => {
        const { intl } = this.props;
        const { formatMessage } = intl;

        const { cycle } = this.state;
        const { on, off } = this.#txt;

        const cycleToggleText: string = formatMessage(
            { defaultMessage: 'Screen Cycling: {status}' },
            { status: cycle.isActive ? on : off },
        );
        let cycleToggleButton: ReactElement = (
            <Button
                key="cycle-toggle-button"
                kind="secondary"
                icon={cycle.isOpen ? IconChevronUp : IconChevronDown}
                onClick={this.#cycleOpenToggle}
            >
                <div className={css.screenCycleButtonContent}>
                    <IconCycle />
                    <span children={cycleToggleText} />
                </div>
            </Button>
        );

        const addSceneText: string = formatMessage({ defaultMessage: 'Add New Scene' });
        let addSceneButton: ReactElement = (
            <Button
                key="add-scene-button"
                kind="primary"
                onClick={this.#openDialogSceneSelect}
                icon={IconAdd}
                children={addSceneText}
            />
        );

        return (
            <Sized<HTMLDivElement>
                render={(ref, size) => {
                    const iconLayout: boolean = !!size && size.width <= 800;
                    if (iconLayout) {
                        cycleToggleButton = (
                            <Button
                                key="cycle-toggle-button"
                                kind="secondary"
                                hasIconOnly
                                title={cycleToggleText}
                                tooltipPosition="bottom"
                                icon={IconCycle}
                                onClick={this.#cycleOpenToggle}
                            />
                        );

                        addSceneButton = (
                            <Button
                                key="add-scene-button"
                                kind="primary"
                                onClick={this.#openDialogSceneSelect}
                                icon={IconAdd}
                                hasIconOnly
                                title={addSceneText}
                                tooltipPosition="bottom"
                            />
                        );
                    }

                    return (
                        <div className={css.headerControls} ref={ref}>
                            <Popover
                                as="div"
                                align="bottom-end"
                                caret={false}
                                isTabTip
                                dropShadow
                                open={cycle.isOpen}
                                ref={this.#cyclePopOverRef}
                            >
                                {cycleToggleButton}

                                <ScreenCyclingConfigForm
                                    cycle={{ value: cycle.isActive, onChange: this.#cycleActiveToggle }}
                                    duration={{
                                        value: cycle.defaultDurationSeconds,
                                        onChange: this.#cycleDurationChange,
                                    }}
                                    transitionEffect={{ value: cycle.effect, onChange: this.#cycleEffectChange }}
                                    render={x => {
                                        if (iconLayout) {
                                            const toggle = this.#cycleOpenToggle;
                                            return (
                                                <Modal
                                                    id={$('cycle-form-modal')}
                                                    open={cycle.isOpen}
                                                    size="sm"
                                                    modalHeading={x.title}
                                                    // Submit
                                                    primaryButtonText={formatMessage({ defaultMessage: 'Save' })}
                                                    onRequestSubmit={toggle}
                                                    // Cancel
                                                    secondaryButtonText={this.#txt.cancel}
                                                    onSecondarySubmit={toggle}
                                                    // Close button
                                                    onRequestClose={toggle}
                                                    children={x.content}
                                                />
                                            );
                                        }
                                        return <PopoverContent className={x.className} children={x.content} />;
                                    }}
                                />
                            </Popover>
                            {addSceneButton}
                        </div>
                    );
                }}
            />
        );
    };

    render() {
        const { intl } = this.props;
        const { scenes } = this.state;

        return (
            <div>
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
                        onMove={this.#sceneListMove}
                        onEdit={this.#sceneListEdit}
                        onClone={this.#sceneListClone}
                        onDelete={this.#sceneListDelete}
                        onToggle={this.#sceneListSetEnabled}
                        onDurationChange={this.#sceneListSetDuration}
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
    transitionEffect: iField<pb.SceneCycleEffect>;
    render(x: { title: string; className: string; content: ReactElement }): ReactElement;
}
function ScreenCyclingConfigForm(props: ScreenCyclingConfigFormProps): ReactElement {
    const intl = useIntl();
    const { formatMessage } = intl;
    const { cycle, duration, transitionEffect, render } = props;

    const cycleDurationOptions: number[] = [10, 20, 30, 40, 50, 60, 120];
    const cycleDurationToString = (value: Maybe<number>): string => {
        if (value == null) return 'N/A';
        const minutes = Math.floor(value / 60);
        const seconds = Math.floor(value - minutes * 60);
        return formatDuration({ minutes, seconds }, { format: ['minutes', 'seconds'] });
    };

    return render({
        title: formatMessage({ defaultMessage: 'Screen Cycling' }),
        className: css.screenCycleContent,
        content: (
            <Form className={css.screenCycleForm}>
                <Toggle
                    id={$('cycle-active')}
                    size="md"
                    toggled={!!cycle.value}
                    onToggle={cycle.onChange}
                    labelText={formatMessage({ defaultMessage: 'Enable Screen Cycling' })}
                    labelA={formatMessage({ defaultMessage: 'Off' })}
                    labelB={formatMessage({ defaultMessage: 'On' })}
                />

                <Dropdown<number>
                    id={$('cycle-duration')}
                    label={formatMessage({ defaultMessage: 'Default Display Duration' })}
                    titleText={formatMessage({ defaultMessage: 'Default Display Duration' })}
                    items={cycleDurationOptions}
                    onChange={x => (x.selectedItem ? duration.onChange(x.selectedItem) : null)}
                    selectedItem={duration.value ?? undefined}
                    itemToString={cycleDurationToString}
                    renderSelectedItem={cycleDurationToString}
                />

                <Dropdown<pb.SceneCycleEffect>
                    id={$('cycle-effect')}
                    label={formatMessage({ defaultMessage: 'Transition Effect' })}
                    titleText={formatMessage({ defaultMessage: 'Transition Effect' })}
                    items={pb.sceneCycleEffects}
                    onChange={x => (x.selectedItem ? transitionEffect.onChange(x.selectedItem) : null)}
                    selectedItem={transitionEffect.value ?? undefined}
                    itemToString={x => pb.sceneCycleEffectToString(intl, x)}
                    renderSelectedItem={x => pb.sceneCycleEffectToString(intl, x)}
                />
            </Form>
        ),
    });
}

export default function DisplayList() {
    const intl = useIntl();
    const navigate = useNavigate();
    return <View intl={intl} navigate={navigate} />;
}
