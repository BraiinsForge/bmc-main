import { Component, createRef, Fragment } from 'react';
import { formatDuration } from 'date-fns';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';

import { Sized } from '@/lib/react';
import { listenDocumentEvent } from '@/lib/dom';
import { Form, getID, type iField, type FormPropsToLocalState } from '@/lib/form';

// App
import * as pb from '@/proto';
import { SceneKind } from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Button, Modal, ModalCustom } from '@/components';
import { Dropdown, Popover, PopoverContent, Toggle } from '@carbon/react';
import {
    Add as IconAdd,
    CarouselHorizontal as IconCycle,
    ChevronDown as IconChevronDown,
    ChevronUp as IconChevronUp,
} from '@carbon/react/icons';
import { FormSceneSelect, SceneOverviewList, FormWidgetClock, type FormWidgetClockProps } from './components';

// Styles
import css from './Display.scss';

interface Props {
    intl: IntlShape;
}

const $ = getID('display').get;
type FormStateClock = FormPropsToLocalState<FormWidgetClockProps>;

interface State {
    scenes: Array<pb.Scene>;
    cycle: {
        isOpen: boolean;
        isActive: boolean;
        defaultDurationSeconds: number;
        effect: pb.SceneCycleEffect;
    };
    add: {
        step:
            | null
            | { kind: 'sceneSelect' }
            | { kind: 'sceneConfig'; sceneKind: pb.SceneKind; sceneVariant: null | pb.SceneVariant };
        clockConf: null | FormStateClock;
    };
}
const getInitialState = (): State => ({
    scenes: [
        {
            id: 0,
            enabled: true,
            durationSeconds: 10,
            kind: pb.SceneKind.combined,
            title: 'Combined Scene',
            description: 'Clock, Clock, Weather, Ticker (BTC-USD)',
        } satisfies pb.Scene,
        {
            id: 1,
            durationSeconds: 11,
            enabled: true,
            kind: pb.SceneKind.image,
            title: 'Image',
            description: 'Your Image',
        } satisfies pb.Scene,
        {
            id: 2,
            enabled: true,
            durationSeconds: 11,
            kind: pb.SceneKind.clock,
            variant: pb.SceneVariantClock.analog_rect,
            title: 'Clock – Analog Rectangular',
            description: 'Horizontal analog layout in a rectangular frame',
        } satisfies pb.Scene,
        {
            id: 3,
            enabled: true,
            durationSeconds: 13,
            kind: pb.SceneKind.ticker,
            variant: pb.SceneVariantTicker.candle,
            title: 'Ticker: Big Price',
            description: 'BTC-USD',
        } satisfies pb.Scene,
        {
            id: 4,
            enabled: true,
            durationSeconds: 14,
            kind: pb.SceneKind.pool,
            title: 'Braiins Pool Stats',
            description: 'account.name',
        } satisfies pb.Scene,
        {
            id: 5,
            durationSeconds: 15,
            enabled: false,
            kind: pb.SceneKind.clock,
            variant: pb.SceneVariantClock.digital_flip,
            title: 'Clock – Flip',
            description: 'Flip-style digital clock with adjustable font weight',
        } satisfies pb.Scene,
    ],
    cycle: {
        isOpen: false,
        isActive: true,
        defaultDurationSeconds: 30,
        effect: pb.SceneCycleEffect.Slide,
    },
    add: {
        step: /* {
            kind: 'sceneConfig',
            sceneKind: SceneKind.clock,
            sceneVariant: null,
        }, */ null,
        clockConf: /* {
        values: {
            clockStyle: pb.ClockStyle.digital1,
            fontStyle: pb.FontStyle.medium,
            showDate: true,
            showSeconds: false,
            showTimezone: true,
            showWeather: true,
            timezone: 'Europe/Berlin',
            weatherLocation: 'Berlin',
        },
        errors: {
            global: [],
            fields: {
                clockStyle: ['Visus camerarius vox est!'],
                fontStyle: ['Nunquam pugna nuptia!'],
                showDate: ['Lapsus satis convertam abactor est!'],
                showSeconds: ['Devatios peregrinatione in brigantium!'],
                showTimezone: ['Est gratis candidatus, cesaris!'],
                showWeather: ['Brodiums mori in cubiculum!'],
                timezone: ['Historia de salvus stella, quaestio valebat!'],
                weatherLocation: ['Abnoba dexter gallus est!'],
            },
        },
    }*/ null,
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

        this.#cycleOpenToggle();
    };
    #windowClickUnsubscribe = (): void => {};

    componentDidMount() {
        this.#windowClickUnsubscribe = listenDocumentEvent({
            name: 'click',
            handler: this.#windowClickHandle,
        }).unsubscribe;
    }
    componentWillUnmount() {
        this.#windowClickUnsubscribe();
    }

    static contextType = AppContext;
    declare context: AppContextType;
    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Display Scenes' }),
            on: formatMessage({ defaultMessage: 'On' }),
            off: formatMessage({ defaultMessage: 'Off' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            addScene: formatMessage({ defaultMessage: 'Add Scene' }),
        };
    }

    #sceneAddStart = (): void => {
        const { add } = getInitialState();
        this.setState({
            add: {
                ...add,
                step: { kind: 'sceneSelect' },
            },
        });
    };
    #sceneAddCancel = (): void => {
        this.setState({ add: { step: null, clockConf: null } });
    };

    #clockGetChangeHandler = <Key extends keyof FormStateClock['values']>(key: Key) => {
        return (value: FormStateClock['values'][Key]) => {
            this.setState(s => ({
                add: {
                    ...s.add,
                    clockConf: {
                        ...s.add.clockConf,
                        errors: null,
                        values: {
                            ...s.add.clockConf?.values,
                            [key]: value,
                        },
                    },
                },
            }));
        };
    };
    #clockGetFieldValue = <Key extends keyof FormStateClock['values']>(
        key: Key,
    ): null | NonNullable<FormStateClock['values'][Key]> => {
        return this.state.add.clockConf?.values?.[key] ?? null;
    };
    #clockGetFieldError = <Key extends keyof FormStateClock['values']>(key: Key): null | string => {
        return pb.renderFieldErrorsAsList(this.state.add.clockConf?.errors?.fields?.[key]);
    };

    #sceneAddSelectKind = (kind: pb.SceneKind): void => {
        this.setState(s => ({
            add: {
                ...s.add,
                step: {
                    kind: 'sceneConfig',
                    sceneKind: kind,
                    sceneVariant: null,
                },
            },
        }));
    };
    #sceneAddRender = (): ReactElement => {
        const { add } = this.state;
        const { intl } = this.props;
        const { formatMessage } = intl;

        const txt = this.#txt;
        const cancel = this.#sceneAddCancel;

        return (
            <Fragment>
                <ModalCustom
                    id={$('add-select-kind-modal')}
                    open={add.step?.kind === 'sceneSelect'}
                    size="md"
                    title={formatMessage({ defaultMessage: 'Add New Display Scene' })}
                    selectorPrimaryFocus="[role=list] [role=button]"
                    onClose={cancel}
                    cancelBodyOverflowShadow
                    children={<FormSceneSelect onClick={this.#sceneAddSelectKind} />}
                />
                <Modal
                    id={$('add-config-modal')}
                    size="sm"
                    modalHeading={formatMessage({ defaultMessage: 'Clock' })}
                    modalLabel={txt.addScene}
                    open={add.step?.kind === 'sceneConfig' && add.step.sceneKind === SceneKind.clock}
                    // Cancel
                    secondaryButtonText={this.#txt.cancel}
                    onSecondarySubmit={cancel}
                    onRequestClose={cancel}
                    // Submit
                    primaryButtonText={txt.addScene}
                >
                    <FormWidgetClock
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
                        showWeather={{
                            value: this.#clockGetFieldValue('showWeather'),
                            error: this.#clockGetFieldError('showWeather'),
                            onChange: this.#clockGetChangeHandler('showWeather'),
                            disabled: false,
                        }}
                        timezone={{
                            value: this.#clockGetFieldValue('timezone'),
                            error: this.#clockGetFieldError('timezone'),
                            onChange: this.#clockGetChangeHandler('timezone'),
                            disabled: false,
                        }}
                        weatherLocation={{
                            value: this.#clockGetFieldValue('weatherLocation'),
                            error: this.#clockGetFieldError('weatherLocation'),
                            onChange: this.#clockGetChangeHandler('weatherLocation'),
                            disabled: false,
                        }}
                    />
                </Modal>
            </Fragment>
        );
    };

    #storeScenes = (scenes: pb.Scene[]): void => this.setState({ scenes });

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
                onClick={this.#sceneAddStart}
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
                                onClick={this.#sceneAddStart}
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
                    <SceneOverviewList scenes={scenes} setScenes={this.#storeScenes} />
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

export default function DisplayPage() {
    const intl = useIntl();
    return <View intl={intl} />;
}
