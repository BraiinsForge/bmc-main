import { Component } from 'react';
import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type Location, useLocation } from 'react-router';

// Lib
import { getTimestamp, validateTime } from '@/lib/time';
import { assertUnreachable } from '@/lib/ts';
import { unloadGuard, Ping, type PingCallback } from '@/lib/dom';

// App
import AppContext, { type AppContextType } from '@/context';
import { store, useStore } from '@/store';
import * as pb from '@/proto';

import {
    SectionGeneral,
    SectionSecurity,
    SectionUpgrade,
    SectionDisplay,
    SectionSoundAndLight,
    Temperature,
    TimeFormat,
    WeekDay,
    type UpgradeFromFeedStatus,
} from './components';
import { InlineNotificationsGroup, Tabs, type TabsProps } from '@/components';
import css from './Settings.scss';
import { setState } from '@/lib/react';

interface Props {
    intl: IntlShape;
    location: Location;
    hasPassword: null | boolean;
}

// The value is used in location hash, so be mindfull of that.
enum Tab {
    general = 'general',
    display = 'display',
    soundAndLight = 'sound-and-light',
    security = 'security',
    updates = 'updates',
}
const validTabs: string[] = Object.values(Tab);

interface LeafState<T> {
    value: null | T;
    errors: Maybe<string[]>;
    isSaving: boolean;
    isLoading: boolean;
}
const leafStateEmpty: Readonly<LeafState<any>> = Object.freeze({
    value: null,
    errors: [],
    isSaving: false,
    isLoading: false,
});
function getEmptyLeafState<T>(value?: Maybe<T>, state?: 'loading' | 'saving'): LeafState<T> {
    const res: LeafState<T> = { ...leafStateEmpty };
    res.value = value ?? null;

    switch (state) {
        case 'saving':
            res.isSaving = true;
            break;

        case 'loading':
            res.isLoading = true;
            break;
    }

    return res;
}

interface State {
    activeTab: Tab;
    globalErrors: Maybe<string[]>;
    data: {
        timezones: ReadonlyArray<pb.Timezone>;
        display: null | pb.DisplaySettingsResponse;
        upgradeInfo: null | pb.CheckForUpgradeResponse;
    };

    genTimezone: LeafState<pb.Timezone>;

    dspBrightness: LeafState<number>;
    dspNightmodeEnabled: LeafState<boolean>;
    dspNightmodeBrightness: LeafState<number>;
    dspNightmodeInterval: LeafState<pb.TimeInterval>;

    soundAndLight: pb.SoundVolumeSettingsResponse;

    secAllowDataCollection: LeafState<boolean>;

    upgradeFromFeedStatus: UpgradeFromFeedStatus;
    upgradeFromFeedErrors: Maybe<string[]>;
}
const defaultNightmodeInterval = pb.create(pb.TimeIntervalSchema, {
    from: '22:00',
    to: '07:00',
});
const getInitialState = (): State => ({
    activeTab: Tab.general,
    globalErrors: null,
    data: {
        timezones: [],
        display: null,
        upgradeInfo: null,
    },

    dspBrightness: getEmptyLeafState(),
    dspNightmodeEnabled: getEmptyLeafState(),
    dspNightmodeBrightness: getEmptyLeafState(),
    dspNightmodeInterval: getEmptyLeafState(),

    soundAndLight: pb.create(pb.SoundVolumeSettingsResponseSchema),

    genTimezone: getEmptyLeafState(),
    secAllowDataCollection: getEmptyLeafState(),

    upgradeFromFeedStatus: { kind: 'idle', upgradeInfo: null },
    upgradeFromFeedErrors: [],
});

class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    #ping: Ping;
    #handlePong: PingCallback = (isOnline, wasOnline) => {
        if (
            // Checking that we actually were offline before coming back
            // avoids potential race-condition with catching pongs
            // of server that did not go down yet.
            //
            // The strict comparison is required because
            // the initial value of `wasOffline` is `null`.
            wasOnline === false &&
            isOnline === true
        ) {
            this.#ping.stop();
            unloadGuard.disable();
            window.location.reload();
        }
    };

    constructor(props: Props) {
        super(props);
        this.state = getInitialState();
        this.#ping = new Ping({
            url: window.location.origin,
            method: 'xhr',
            onPong: this.#handlePong,
            interval: 500,
        });
    }

    componentDidMount = () => this.#mount();
    componentWillUnmount = () => pb.abort.all(this);

    #mount = debounce(() => {
        this.#syncTabs();
        this.#fetchData();
    }, 150);
    #syncTabs = () => {
        const { location } = this.props;
        const maybeTabHash = location.hash.slice(1);
        if (!maybeTabHash) this.#tabChange(Tab.general);
        else if (validTabs.includes(maybeTabHash)) this.#tabChange(maybeTabHash as Tab);
    };

    #noop = () => {};
    // Forcing the external ID usage ensures that we won't spam the user with repeated notifications.
    #notifySuccess = (message: string): void => {
        this.context.notify('success', message, { id: 'settings-saved', timeoutSeconds: 3 });
    };
    #notifyError = (message: string, tag: string): void => {
        this.context.notify('error', message, { id: tag, timeoutSeconds: 3 });
    };
    #fetchData = async (): Promise<void> => {
        const q = [this.#upgradesFeedCheck(), this.#fetchSystemInfo(), this.#displayFetch(), this.#soundLightFetch()];
        await Promise.allSettled(q);
    };

    private fetchSystemInfoAbort = pb.abort.get();
    #fetchSystemInfo = async (): Promise<void> => {
        try {
            const { signal } = this.fetchSystemInfoAbort.replace();
            const [{ timezone }, { timezones }] = await Promise.all([
                pb.rpc.sys.getTimezone({}, { signal }),
                pb.rpc.sys.getTimezoneList({}, { signal }),
            ]);
            this.setState(s => ({
                data: { ...s.data, timezones },
                genTimezone: getEmptyLeafState(timezone),
            }));
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({ globalErrors: pb.collectAllErrors($) });
        }
    };

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Settings' }),
        };
    }

    get #tabs(): TabsProps<Tab>['tabs'] {
        const { formatMessage } = this.props.intl;
        return [
            {
                key: Tab.general,
                label: formatMessage({ defaultMessage: 'General' }),
            },
            {
                key: Tab.display,
                label: formatMessage({ defaultMessage: 'Display' }),
            },
            {
                key: Tab.soundAndLight,
                label: formatMessage({ defaultMessage: 'Sound & Light' }),
            },
            {
                key: Tab.security,
                label: formatMessage({ defaultMessage: 'Security' }),
            },
            {
                key: Tab.updates,
                label: formatMessage({ defaultMessage: 'Upgrades' }),
            },
        ];
    }
    #tabChange = (tab: Tab): void => {
        if (this.state.activeTab === tab) return;
        this.setState({ activeTab: tab });
        window.history.replaceState(null, '', `#${tab}`);
    };

    //
    // General
    //

    private generalSetTimezoneAbort = pb.abort.get();
    #generalSetTimezone = async (value: pb.Timezone): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { genTimezone } = this.state;

        // No sense in moving ahead if we don't yet know
        // the current timezone or if it's already set.
        if (genTimezone.value == null || value.id === genTimezone.value?.id) return;

        try {
            const { signal } = this.generalSetTimezoneAbort.replace();
            await setState(this, s => ({ genTimezone: getEmptyLeafState(s.genTimezone.value, 'saving') }));
            await pb.rpc.sys.setTimezone({ id: value.id }, { signal });
            this.#notifySuccess(formatMessage({ defaultMessage: 'Timezone changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.collectAllErrors($);
            this.setState(s => ({ genTimezone: { ...s.genTimezone, errors } }));
        } finally {
            await this.#fetchSystemInfo();
            this.setState(s => ({ genTimezone: getEmptyLeafState(s.genTimezone.value) }));
        }
    };
    #generalFactoryReset = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            await pb.rpc.sys.factoryReset({});
            this.#notifySuccess(formatMessage({ defaultMessage: 'Factory reset complete!' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Unknown error!' });
            this.#notifyError(msg, 'factory-reset-error');
        }
    };
    #generalRender = (): ReactNode => {
        const { data, genTimezone } = this.state;

        return (
            <SectionGeneral
                timeFormat={{
                    value: TimeFormat.twentyFour,
                    disabled: true,
                    onChange: this.#noop,
                }}
                secondsInStatusbar={{
                    value: false,
                    disabled: true,
                    onChange: this.#noop,
                }}
                timezone={{
                    value: genTimezone.value,
                    error: pb.renderFieldErrorsAsList(genTimezone.errors),
                    items: data.timezones,
                    disabled: genTimezone.isLoading || genTimezone.isSaving,
                    onChange: this.#generalSetTimezone,
                }}
                dateFormat={{
                    value: 'DMY_SLASH',
                    disabled: true,
                    onChange: this.#noop,
                }}
                firstWeekDay={{
                    value: WeekDay.Monday,
                    disabled: true,
                    onChange: this.#noop,
                }}
                temperature={{
                    value: Temperature.C,
                    disabled: true,
                    onChange: this.#noop,
                }}
                numberFormat={{
                    value: 'spaceAndComma',
                    disabled: true,
                    onChange: this.#noop,
                }}
                onFactoryReset={this.#generalFactoryReset}
            />
        );
    };

    //
    // Display
    //

    private displayFetchAbort = pb.abort.get();
    #displayFetch = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.displayFetchAbort.replace();
            const d = await pb.rpc.config.getDisplaySettings({}, { signal });
            this.setState(s => ({
                data: {
                    ...s.data,
                    display: d,
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string = formatMessage({ defaultMessage: 'Failed to load display settings!' });
            this.#notifyError(msg, 'display-settings-load-error');
        }
    };
    private displaySetBrightnessAbort = pb.abort.get();
    #displaySetBrightness = debounce(async (value: Maybe<number>): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const currentValue = this.state.data.display?.brightness?.value ?? null;
        if (value == null || value === currentValue) return;

        try {
            const { signal } = this.displaySetBrightnessAbort.replace();
            await pb.rpc.config.setBrightness({ value }, { signal });
            this.#notifySuccess(formatMessage({ defaultMessage: 'Brightness Saved' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState(s => ({
                dspBrightness: {
                    ...s.dspBrightness,
                    errors: pb.collectAllErrors($) ?? [
                        formatMessage({ defaultMessage: 'Failed to save the brightness!' }),
                    ],
                },
            }));
        }
    }, 600);
    private displaySetNightmodeEnabledAbort = pb.abort.get();
    #displaySetNightmodeEnabled = async (newEnabled: Maybe<boolean>): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { display } = this.state.data;

        // Catch the display === null
        if (!display) {
            notify('error', formatMessage({ defaultMessage: 'Display settings not loaded yet!' }));
            return;
        }

        // Сurrent state snapshot
        const currentEnabled: boolean | null = display?.nightmodeEnabled ?? null;
        const currentInterval: pb.TimeInterval | undefined = display?.nightmodeInterval;
        const hasInterval: boolean = currentInterval != null;
        const nextInterval: pb.TimeInterval = currentInterval ?? defaultNightmodeInterval;

        // Ignore redundant updates
        if (newEnabled == null || newEnabled === currentEnabled) return;

        try {
            const { signal } = this.displaySetNightmodeEnabledAbort.replace();
            // Optimistic update (clear errors)
            this.setState(s => ({
                dspNightmodeEnabled: {
                    ...s.dspNightmodeEnabled,
                    value: newEnabled,
                    isSaving: true,
                    errors: null,
                },
                dspNightmodeInterval: { ...s.dspNightmodeInterval, errors: null },
                dspNightmodeBrightness: { ...s.dspNightmodeBrightness, errors: null },
            }));

            // Persist changes
            if (newEnabled === false || hasInterval) {
                // Case 1: disable nightmode or reuse existing interval
                // only update enabled flag
                await pb.rpc.config.setNightmodeEnabled({ value: newEnabled }, { signal });
            } else {
                // Case 2: first-time enable without interval
                // enable nightmode AND initialize default interval
                await Promise.all([
                    pb.rpc.config.setNightmodeEnabled({ value: newEnabled }, { signal }),
                    pb.rpc.config.setNightmodeInterval(defaultNightmodeInterval, { signal }),
                ]);
            }

            // Commit saved values

            const updatedDisplay: pb.DisplaySettingsResponse = { ...display, nightmodeEnabled: newEnabled };

            this.setState(s => ({
                data: {
                    ...s.data,
                    display: updatedDisplay,
                },
                dspNightmodeEnabled: {
                    ...s.dspNightmodeEnabled,
                    value: newEnabled,
                    isSaving: false,
                },
                dspNightmodeInterval: {
                    ...s.dspNightmodeInterval,
                    value: nextInterval,
                },
            }));

            notify('success', formatMessage({ defaultMessage: 'Night Mode Saved' }));
        } catch ($) {
            // Error handling
            if (pb.abort.is($)) return;
            this.setState(s => ({
                dspNightmodeEnabled: {
                    ...s.dspNightmodeEnabled,
                    value: s.data.display?.nightmodeEnabled ?? false,
                    isSaving: false,
                    errors: pb.collectAllErrors($) ?? [
                        formatMessage({ defaultMessage: 'Failed to save the Night Mode!' }),
                    ],
                },
            }));
        }
    };
    private displaySetNightmodeBrightnessAbort = pb.abort.get();
    #displaySetNightmodeBrightness = debounce(async (value: Maybe<number>): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;

        // Сurrent state snapshot
        const currentValue: number | null = this.state.data.display?.brightnessNightmode?.value ?? null;
        // Ignore redundant updates
        if (value == null || value === currentValue) return;

        try {
            // Persist changes
            const { signal } = this.displaySetNightmodeBrightnessAbort.replace();
            await pb.rpc.config.setBrightnessNightmode({ value }, { signal });
            notify('success', formatMessage({ defaultMessage: 'Night mode brightness saved' }));
        } catch ($) {
            // Error handling
            if (pb.abort.is($)) return;
            this.setState(s => ({
                dspNightmodeBrightness: {
                    ...s.dspNightmodeBrightness,
                    errors: pb.collectAllErrors($) ?? [
                        formatMessage({ defaultMessage: 'Failed to save the night mode brightness!' }),
                    ],
                },
            }));
        }
    }, 600);
    #handleNightmodeIntervalChange = (value: pb.TimeInterval) => {
        //Update UI values and clear errors
        this.setState(s => ({
            dspNightmodeInterval: {
                ...s.dspNightmodeInterval,
                value: { ...s.dspNightmodeInterval.value, ...value },
                errors: null,
            },
        }));

        this.displaySetNightmodeInterval(value);
    };
    private displaySetNightmodeIntervalAbort = pb.abort.get();
    private displaySetNightmodeInterval = debounce(async (value: pb.TimeInterval): Promise<void> => {
        const { notify } = this.context;
        const { formatMessage } = this.props.intl;
        const { display } = this.state.data;

        // Catch the display === null
        if (!display) {
            notify('error', formatMessage({ defaultMessage: 'Display settings not loaded yet!' }));
            return;
        }

        // Сurrent state snapshot
        const currentValue: pb.TimeInterval | null = display?.nightmodeInterval ?? null;
        const validationErrors: string[] = [];

        // Set saving state
        this.setState(s => ({
            dspNightmodeInterval: {
                ...s.dspNightmodeInterval,
                isSaving: true,
            },
        }));
        // Ignore redundant updates or validate time interval

        if (value.from === currentValue?.from && value.to === currentValue?.to) return;

        if (!value.from || !value.to)
            validationErrors.push(formatMessage({ defaultMessage: 'Time interval must be set!' }));

        if (value.from === value.to)
            validationErrors.push(formatMessage({ defaultMessage: 'Start and end time cannot be the same!' }));

        if (!validateTime(value.from) || !validateTime(value.to))
            validationErrors.push(formatMessage({ defaultMessage: 'Time must be in HH:MM format!' }));

        if (validationErrors.length > 0) {
            this.setState(s => ({
                dspNightmodeInterval: {
                    ...s.dspNightmodeInterval,
                    errors: [...validationErrors],
                },
            }));
            return;
        }

        try {
            const { signal } = this.displaySetNightmodeIntervalAbort.replace();

            await pb.rpc.config.setNightmodeInterval(value, { signal });

            const updatedDisplay: pb.DisplaySettingsResponse = { ...display, nightmodeInterval: value };

            this.setState(s => ({
                data: {
                    ...s.data,
                    display: updatedDisplay,
                },
                dspNightmodeInterval: {
                    ...s.dspNightmodeInterval,
                    isSaving: false,
                },
            }));

            notify('success', formatMessage({ defaultMessage: 'Night mode time interval saved' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState(s => ({
                dspNightmodeInterval: {
                    ...s.dspNightmodeInterval,
                    isSaving: false,
                    errors: pb.collectAllErrors($) ?? [
                        formatMessage({ defaultMessage: 'Failed to save the night mode time interval!' }),
                    ],
                },
            }));
        }
    }, 800);
    #displayRender = (): ReactNode => {
        const { dspBrightness, data, dspNightmodeEnabled, dspNightmodeBrightness, dspNightmodeInterval } = this.state;
        const { display } = data;

        const brightness: pb.BrightnessInfo | undefined = display?.brightness;
        const nightmodeEnabled: boolean | undefined = display?.nightmodeEnabled;
        const brightnessNightmode: pb.BrightnessInfo | undefined = display?.brightnessNightmode;
        const nightmodeInterval: pb.TimeInterval | undefined = display?.nightmodeInterval;

        return (
            <SectionDisplay
                brightness={{
                    value: dspBrightness.value ?? brightness?.value ?? null,
                    min: brightness?.min,
                    max: brightness?.max,
                    step: brightness?.step,
                    error: pb.renderFieldErrorsAsList(dspBrightness.errors),
                    disabled: !brightness,
                    onChange: this.#displaySetBrightness,
                }}
                // Night
                nightEnabled={{
                    value: dspNightmodeEnabled.value ?? nightmodeEnabled ?? false,
                    disabled: false,
                    onChange: this.#displaySetNightmodeEnabled,
                }}
                nightBrightness={{
                    value: dspNightmodeBrightness.value ?? brightnessNightmode?.value ?? null,
                    min: brightnessNightmode?.min,
                    max: brightnessNightmode?.max,
                    step: brightnessNightmode?.step,
                    error: pb.renderFieldErrorsAsList(dspNightmodeBrightness.errors),
                    disabled: !(dspNightmodeEnabled.value ?? nightmodeEnabled),
                    onChange: this.#displaySetNightmodeBrightness,
                }}
                nightInterval={{
                    value: dspNightmodeInterval.value ?? nightmodeInterval ?? null,
                    error: pb.renderFieldErrorsAsList(dspNightmodeInterval.errors),
                    disabled: !(dspNightmodeEnabled.value ?? nightmodeEnabled),
                    onChange: this.#handleNightmodeIntervalChange,
                }}
                nightNotify={{
                    value: true,
                    disabled: true,
                    onChange: this.#noop,
                }}
                // Location
                nightUseLocation={{
                    value: true,
                    disabled: true,
                    onChange: this.#noop,
                }}
                nightLocation={{
                    value: 'Prague, Czechia',
                    disabled: true,
                    onChange: this.#noop,
                }}
                onLocationDetect={this.#noop}
            />
        );
    };

    //
    // Sound & Light
    //

    private soundLightFetcDataAbort = pb.abort.get();
    #soundLightFetch = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { signal } = this.soundLightFetcDataAbort.replace();
            const soundAndLight = await pb.rpc.config.getSoundVolumeSettings({}, { signal });
            this.setState({ soundAndLight });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load sound settings!' });
            this.#notifyError(msg, 'sound-settings-load-error');
        }
    };
    #soundLightSetVolume = debounce(async (value: number): Promise<void> => {
        if (value === this.state.soundAndLight.volume?.value) return;
        const { formatMessage } = this.props.intl;

        try {
            // Positive update
            this.setState(s => ({
                soundAndLight: {
                    ...s.soundAndLight,
                    volume: pb.create(pb.SoundVolumeSchema, {
                        min: s.soundAndLight.volume?.min,
                        max: s.soundAndLight.volume?.max,
                        step: s.soundAndLight.volume?.step,
                        value,
                    }),
                },
            }));

            // Submit
            await pb.rpc.config.setSoundVolume({ value });
            this.#notifySuccess(formatMessage({ defaultMessage: 'Sound volume saved' }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save the sound volume!' });
            this.#notifyError(msg, 'sound-volume-save-error');
        } finally {
            await this.#soundLightFetch();
        }
    }, 200);
    #soundLightSetVolumeNight = debounce(async (value: number): Promise<void> => {
        if (value === this.state.soundAndLight.volumeNightmode?.value) return;
        const { formatMessage } = this.props.intl;

        try {
            // Positive update
            this.setState(s => ({
                soundAndLight: {
                    ...s.soundAndLight,
                    volumeNightmode: pb.create(pb.SoundVolumeSchema, {
                        min: s.soundAndLight.volumeNightmode?.min,
                        max: s.soundAndLight.volumeNightmode?.max,
                        step: s.soundAndLight.volumeNightmode?.step,
                        value,
                    }),
                },
            }));

            // Submit
            await pb.rpc.config.setSoundVolumeNightmode({ value });
            this.#notifySuccess(formatMessage({ defaultMessage: 'Night mode sound volume saved' }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save the night mode sound volume!' });
            this.#notifyError(msg, 'sound-volume-save-error');
        } finally {
            await this.#soundLightFetch();
        }
    }, 200);
    // #soundLightSetNotificationLightsEnabled = debounce(async (value: boolean): Promise<void> => {
    //     const { formatMessage } = this.props.intl;
    //     const { notify } = this.context;
    //
    //     try {
    //         await pb.rpc.config;
    //     } catch ($) {
    //         let msg = pb.collectAllErrorsAsFormattedList($);
    //         msg ||= formatMessage({ defaultMessage: 'Failed to save the sound volume!' });
    //         this.#notifyError(msg, 'sound-volume-save-error');
    //     }
    // }, 200);
    #soundLightRender = (): ReactNode => {
        const { volume, volumeNightmode } = this.state.soundAndLight;

        return (
            <SectionSoundAndLight
                soundVolume={{
                    min: volume?.min ?? 0,
                    max: volume?.max ?? 100,
                    step: volume?.step ?? 1,
                    value: volume?.value ?? 0,
                    onChange: this.#soundLightSetVolume,
                }}
                soundVolumeNight={{
                    min: volumeNightmode?.min ?? 0,
                    max: volumeNightmode?.max ?? 100,
                    step: volumeNightmode?.step ?? 1,
                    value: volumeNightmode?.value ?? 0,
                    onChange: this.#soundLightSetVolumeNight,
                }}
                // alarmAndNotifyVolume={{ value: 65, onChange: this.#noop }}
                ledNotifyEnabled={{
                    value: true,
                    disabled: true,
                    onChange: this.#noop,
                }}
            />
        );
    };

    //
    // Security
    //

    #secOnPasswordChange = async (data: pb.ChangePasswordRequest): Promise<void> => {
        const { intl, hasPassword } = this.props;

        // Abort if we got called wrongly
        if (hasPassword !== true) {
            return this.setState({
                globalErrors: [intl.formatMessage({ defaultMessage: 'Password is not set!' })],
            });
        }

        await pb.rpc.sys.changePassword(data);
        await store.fetchSessionInfo();
    };
    #secOnPasswordRemove = async (data: pb.RemovePasswordRequest): Promise<void> => {
        const { intl, hasPassword } = this.props;

        // Abort if we got called wrongly
        if (hasPassword !== true) {
            return this.setState({
                globalErrors: [intl.formatMessage({ defaultMessage: 'Password is not set!' })],
            });
        }

        await pb.rpc.sys.removePassword(data);
        await store.fetchSessionInfo();
    };
    #secOnPasswordCreate = async (data: pb.CreatePasswordRequest): Promise<void> => {
        const { intl, hasPassword } = this.props;

        // Abort if we got called wrongly
        if (hasPassword !== false) {
            return this.setState({
                globalErrors: [intl.formatMessage({ defaultMessage: 'Password is already set!' })],
            });
        }

        await pb.rpc.sys.createPassword(data);
        await store.fetchSessionInfo();
    };
    #secActions = {
        onPasswordChange: this.#secOnPasswordChange,
        onPasswordRemove: this.#secOnPasswordRemove,
        onPasswordCreate: this.#secOnPasswordCreate,
    };
    #secRender = (): ReactNode => {
        const { hasPassword } = this.props;
        const { secAllowDataCollection } = this.state;

        return (
            <SectionSecurity
                hasPassword={hasPassword}
                actions={this.#secActions}
                dataCollection={{
                    value: secAllowDataCollection.value,
                    disabled: true, // FIXME: secAllowDataCollection.isLoading || secAllowDataCollection.isSaving,
                    // FIXME: Implement the setter
                    onChange: this.#noop,
                }}
            />
        );
    };

    //
    // Upgrades
    //

    private upgradesFeedCheckAbort = pb.abort.get();
    #upgradesFeedCheck = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        await setState(this, { upgradeFromFeedStatus: { kind: 'checking-upgrade', upgradeInfo: null } });

        try {
            const { signal } = this.upgradesFeedCheckAbort.replace();
            const upgradeInfo = await pb.rpc.upgrade.checkForUpgrade({}, { signal });

            // Upgrade available
            if (upgradeInfo.latestRelease) {
                this.setState(s => ({
                    data: { ...s.data, upgradeInfo },
                    upgradeFromFeedStatus: {
                        kind: 'upgrade-available',
                        upgradeInfo,
                    },
                }));
            }

            // Up to date
            else {
                this.setState(s => ({
                    data: { ...s.data, upgradeInfo },
                    upgradeFromFeedStatus: { kind: 'up-to-date', upgradeInfo: null },
                }));
            }
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors = pb.collectAllErrors($);
            const message = formatMessage({ defaultMessage: 'Failed to check for upgrade!' });
            this.#notifyError(message, 'upgrade-check-error');

            this.setState(s => ({
                data: { ...s.data, upgradeInfo: null },
                upgradeFromFeedErrors: errors,
                upgradeFromFeedStatus: { kind: 'idle', upgradeInfo: null },
            }));
        }
    };

    private upgradesFeedDownloadAbort = pb.abort.get();
    #upgradesFeedDownload = async (hash: string): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { upgradeInfo } = this.state.data;

        const setBackWithError = (error: string[]) => {
            unloadGuard.disable();
            this.setState(
                {
                    upgradeFromFeedStatus: { kind: 'idle', upgradeInfo: null },
                    upgradeFromFeedErrors: error,
                },
                this.#upgradesFeedCheck,
            );
        };
        if (!upgradeInfo) return setBackWithError([formatMessage({ defaultMessage: 'Upgrade info not available!' })]);
        unloadGuard.enable();

        try {
            const { signal } = this.upgradesFeedDownloadAbort.replace();

            const stream = pb.rpc.upgrade.downloadFirmware({ hash }, { signal });
            for await (const msg of stream) {
                const data = msg.state;
                if (!data.case) {
                    return setBackWithError([formatMessage({ defaultMessage: 'Invalid system upgrade response!' })]);
                }

                switch (data.case) {
                    case 'downloadProgress':
                        this.setState({
                            upgradeFromFeedStatus: {
                                kind: 'downloading',
                                upgradeInfo,
                                downloadProgress: data.value,
                            },
                        });
                        break;

                    case 'downloadFinished':
                        unloadGuard.disable();
                        await setState(this, {
                            upgradeFromFeedStatus: {
                                kind: 'installing',
                                upgradeInfo,
                                startTime: getTimestamp(),
                            },
                        });
                        this.#upgradesFeedConfirm(data.value.hash);
                        break;

                    default:
                        assertUnreachable(data, 'upgrade download progress');
                }
            }
        } catch ($) {
            unloadGuard.disable();
            if (pb.abort.is($)) return;
            const error = pb.collectAllErrorsAsFormattedList($);
            const message = formatMessage({ defaultMessage: 'Unexpected error: {error}' }, { error });
            this.#notifyError(message, 'upgrade-download-error');
        }
    };

    private abortFeedUpgrade = pb.abort.get();
    #upgradesFeedConfirm = async (hash: string): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { upgradeInfo } = this.state.data;

        const setBackWithError = (error: string[]) => {
            this.setState(
                s => ({
                    data: { ...s.data, upgradeInfo: null },
                    upgradeFromFeedErrors: error,
                    upgradeFromFeedStatus: { kind: 'idle', upgradeInfo: null },
                }),
                this.#upgradesFeedCheck,
            );
        };
        if (!upgradeInfo) return setBackWithError([formatMessage({ defaultMessage: 'Upgrade info not available!' })]);

        // When the installation starts, the process leads to either
        //  - an error
        //  - a success with a server restart
        //
        // This means that this one needs to be removed either on the error
        // or when reloading the page after the server restart
        unloadGuard.enable();

        try {
            await setState(this, {
                upgradeFromFeedStatus: {
                    kind: 'installing',
                    upgradeInfo,
                    startTime: getTimestamp(),
                },
            });

            const { signal } = this.abortFeedUpgrade.replace();
            await pb.rpc.upgrade.upgrade({ hash }, { signal });

            await setState(this, {
                upgradeFromFeedErrors: null,
                upgradeFromFeedStatus: {
                    kind: 'restarting',
                    upgradeInfo: null,
                    startTime: Math.floor(Date.now() / 1e3),
                },
            });
            this.#ping.start();
        } catch ($) {
            unloadGuard.disable();
            if (pb.abort.is($)) return;
            setBackWithError(
                pb.collectAllErrors($) ?? [formatMessage({ defaultMessage: 'Failed to upgrade the system!' })],
            );
        }
    };

    #updatesToggle = (enabled: boolean): void => console.log(enabled);
    #updatesRender = (): ReactNode => {
        const { upgradeFromFeedStatus, upgradeFromFeedErrors } = this.state;
        return (
            <SectionUpgrade
                automaticUpgrades={{
                    value: true,
                    onChange: this.#updatesToggle,
                    disabled: true,
                }}
                versionCurrent="24.04.1"
                status={upgradeFromFeedStatus}
                errors={upgradeFromFeedErrors}
                onCheckUpdates={this.#upgradesFeedCheck}
                onDownload={this.#upgradesFeedDownload}
            />
        );
    };

    render() {
        const { activeTab, globalErrors } = this.state;
        const { title } = this.#txt;

        let content: ReactNode;
        switch (activeTab) {
            case Tab.general:
                content = this.#generalRender();
                break;

            case Tab.display:
                content = this.#displayRender();
                break;

            case Tab.soundAndLight:
                content = this.#soundLightRender();
                break;

            case Tab.security:
                content = this.#secRender();
                break;

            case Tab.updates:
                content = this.#updatesRender();
                break;

            default:
                assertUnreachable(activeTab, 'settings: active tab');
        }

        return (
            <div className={css.root}>
                <Helmet title={title} />
                <h1 className={css.title} children={title} />

                <Tabs tabs={this.#tabs} activeTab={activeTab} onChange={this.#tabChange} className={css.tabs} />
                <InlineNotificationsGroup
                    kind="error"
                    items={globalErrors}
                    stretch
                    theme="inverse"
                    style={{ marginBottom: '1rem' }}
                />
                <div className={css.content} children={content} />
            </div>
        );
    }
}

export default function () {
    const intl = useIntl();
    const location = useLocation();
    const hasPassword = useStore(x => x.state.sessionInfo.hasPassword);
    return <View intl={intl} location={location} hasPassword={hasPassword} />;
}
