import { Component } from 'react';
import { debounce, isEqual } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type Location, useLocation } from 'react-router';

// Lib
import { setState } from '@/lib/react';
import type { iField } from '@/lib/form';
import { toast } from '@/lib/toast';
import { assertUnreachable } from '@/lib/ts';
import { getTimestamp, validateTime } from '@/lib/time';
import { unloadGuard, Ping, type PingCallback, downloadURL } from '@/lib/dom';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { store, useStore } from '@/store';
import AppContext, { type AppContextType } from '@/context';

// Components
import {
    SectionGeneral,
    SectionSecurity,
    SectionUpgrade,
    SectionDisplay,
    SectionSoundAndLight,
    type UpgradeFromFeedStatus,
} from './components';
import { InlineNotificationsGroup, Tabs, type TabsProps } from '@/components';

// Styles
import css from './Settings.scss';

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

interface FieldState<T> {
    value: null | T;
    errors: Maybe<string[]>;
    isSaving: boolean;
    isLoading: boolean;
}
type ValueOrPatcher<T> = T | ((value: T) => T);

const FIELD_STATE_DEFAULT: Readonly<FieldState<any>> = Object.freeze({
    value: null,
    errors: [],
    isSaving: false,
    isLoading: false,
});
function getFieldStateDefault<T>(value?: Maybe<T>, state?: 'loading' | 'saving'): FieldState<T> {
    const res: FieldState<T> = { ...FIELD_STATE_DEFAULT };
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

    // Data that does not fit neatly into the form fields.
    data: {
        metadata: null | pb.Metadata;
        timezones: ReadonlyArray<pb.Timezone>;
        upgradeInfo: null | pb.CheckForUpgradeResponse;
        displayNightmodeIntervalBackendValue: null | pb.TimeInterval;
    };

    // Strictly just the FieldState objects to allow us
    // to abstract away a lot of the interactions and getters
    values: {
        // General
        timeFormat: FieldState<pb.TimeFormat>;
        dateFormat: FieldState<pb.DateFormat>;
        numberFormat: FieldState<pb.NumberFormat>;
        dataCollection: FieldState<boolean>;
        // showSecondsStatusBar: LeafState<boolean>;
        firstDayOfWeek: FieldState<pb.Weekday>;
        temperatureUnit: FieldState<pb.TemperatureUnit>;
        unitSystem: FieldState<pb.UnitSystem>;
        timezone: FieldState<pb.Timezone>;

        // Sound & Light
        volume: FieldState<pb.SoundVolume>;
        volumeNightmode: FieldState<pb.SoundVolume>;
        enableBootSound: FieldState<boolean>;
        enableLedNotifications: FieldState<boolean>;
        enableLedNotificationsNightmode: FieldState<boolean>;

        // Display
        displayBrightness: FieldState<pb.BrightnessInfo>;
        displayNightmodeEnabled: FieldState<boolean>;
        displayNightmodeBrightness: FieldState<pb.BrightnessInfo>;
        displayNightmodeInterval: FieldState<pb.TimeInterval>;
    };

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
        metadata: null,
        timezones: [],
        upgradeInfo: null,
        displayNightmodeIntervalBackendValue: null,
    },
    values: {
        // General
        timeFormat: getFieldStateDefault(),
        dateFormat: getFieldStateDefault(),
        numberFormat: getFieldStateDefault(),
        dataCollection: getFieldStateDefault(),
        // showSecondsStatusBar: getEmptyLeafState(),
        firstDayOfWeek: getFieldStateDefault(),
        temperatureUnit: getFieldStateDefault(),
        unitSystem: getFieldStateDefault(),
        timezone: getFieldStateDefault(),

        // Sound & Light
        volume: getFieldStateDefault(),
        volumeNightmode: getFieldStateDefault(),
        enableBootSound: getFieldStateDefault(),
        enableLedNotifications: getFieldStateDefault(),
        enableLedNotificationsNightmode: getFieldStateDefault(),

        // Display
        displayBrightness: getFieldStateDefault(),
        displayNightmodeEnabled: getFieldStateDefault(),
        displayNightmodeBrightness: getFieldStateDefault(),
        displayNightmodeInterval: getFieldStateDefault(),
    } satisfies Record<string, FieldState<any>>,

    upgradeFromFeedStatus: { kind: 'idle', upgradeInfo: null },
    upgradeFromFeedErrors: [],
});

const noop = (): void => {};

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
    #fetchData = async (): Promise<void> => {
        const q = [
            this.#generalFetch(),
            this.#fetchSystemInfo(),
            this.#upgradesFeedCheck(),
            this.#displayFetch(),
            this.#soundLightFetch(),
        ];
        await Promise.allSettled(q);
    };

    private fetchSystemInfoAbort = pb.abort.get();
    #fetchSystemInfo = async (): Promise<void> => {
        try {
            const { signal } = this.fetchSystemInfoAbort.replace();
            const [{ timezone }, { timezones }, metadata] = await Promise.all([
                pb.rpc.sys.getTimezone({}, { signal }),
                pb.rpc.sys.getTimezoneList({}, { signal }),
                pb.rpc.meta.getMetadata({}, { signal }),
            ]);
            this.setState(s => ({
                data: { ...s.data, timezones, metadata },
                values: { ...s.values, timezone: getFieldStateDefault(timezone) },
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

    private generalFetchAbort = pb.abort.get();
    #generalFetch = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        try {
            const { signal } = this.generalFetchAbort.replace();
            const d = await pb.rpc.config.getGeneralSettingsData({}, { signal });
            this.setState(s => ({
                values: {
                    ...s.values,
                    timeFormat: getFieldStateDefault(d.timeFormat),
                    dateFormat: getFieldStateDefault(d.dateFormat),
                    numberFormat: getFieldStateDefault(d.numberFormat),
                    dataCollection: getFieldStateDefault(d.dataCollection),
                    showSecondsStatusBar: getFieldStateDefault(d.showSecondsStatusBar),
                    firstDayOfWeek: getFieldStateDefault(d.firstDayOfWeek),
                    temperatureUnit: getFieldStateDefault(d.temperatureUnit),
                    unitSystem: getFieldStateDefault(d.unitSystem),
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load general settings!' });

            toast.error(msg);
        }
    };

    #setField = <Field extends keyof State['values']>(
        field: Field,
        value: ValueOrPatcher<State['values'][Field]>,
    ): Promise<void> => {
        return setState(this, state => ({
            values: {
                ...state.values,
                [field]: typeof value === 'function' ? value(state.values[field]) : value,
            },
        }));
    };
    #setFieldAttr = <Field extends keyof State['values'], Attr extends keyof FieldState<any>>(
        field: Field,
        attr: Attr,
        value: ValueOrPatcher<State['values'][Field][Attr]>,
    ): Promise<void> => {
        return this.#setField(field, fld => ({
            ...fld,
            [attr]: typeof value === 'function' ? value(fld[attr]) : value,
        }));
    };
    #getFieldStruct<Value, Extra extends Rec = Rec>(
        state: FieldState<Value>,
        changeHandler: (value: Value) => void,
        extra?: Extra,
    ): iField<Value> & Extra {
        return Object.assign(
            {
                value: state.value,
                error: pb.renderFieldErrorsAsList(state.errors),
                disabled: state.isLoading || state.isSaving,
                onChange: changeHandler,
            },
            extra,
        );
    }

    private generalSetTimezoneAbort = pb.abort.get();
    #generalSetTimezone = async (value: pb.Timezone): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { timezone } = this.state.values;

        // No sense in moving ahead if we don't yet know
        // the current timezone or if it's already set.
        if (timezone.value == null || value.id === timezone.value?.id) return;

        try {
            // Optimistic update & saving flag
            await this.#setField('timezone', s => ({ ...s, value, isSaving: true }));

            const { signal } = this.generalSetTimezoneAbort.replace();
            await pb.rpc.sys.setTimezone({ id: value.id }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Timezone changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors = pb.collectAllErrors($);
            this.#setFieldAttr('timezone', 'errors', errors);
        } finally {
            await this.#fetchSystemInfo();
            this.#setField('timezone', s => getFieldStateDefault(s.value));
        }
    };

    private generalSetTimeFormatAbort = pb.abort.get();
    #generalSetTimeFormat = async (value: pb.TimeFormat): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('timeFormat', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetTimeFormatAbort.replace();
            await pb.rpc.config.setTimeFormat({ timeFormat: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Time format changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let error = pb.collectAllErrorsAsFormattedList($);
            error ||= formatMessage({ defaultMessage: 'Failed to save SecondsInStatusbar' });
            this.#setFieldAttr('timeFormat', 'errors', [error]);
        } finally {
            await this.#generalFetch();
            this.#setField('timeFormat', s => getFieldStateDefault(s.value));
        }
    };

    // private generalSetSecondsInStatusbarAbort = pb.abort.get();
    // #generalSetSecondsInStatusbar = async (value: boolean): Promise<void> => {
    //     const { formatMessage } = this.props.intl;
    //
    //     try {
    //         // Optimistic update & saving flag
    //         this.#generalSetField('showSecondsStatusBar', s => ({ ...s, value, isSaving: true }));
    //
    //         // Submit
    //         const { signal } = this.generalSetSecondsInStatusbarAbort.replace();
    //         await pb.rpc.config.showSecondsInStatusBar({ value }, { signal });
    //
    //         toast.success(formatMessage({ defaultMessage: 'Seconds in status bar changed' }));
    //     } catch ($) {
    //         if (pb.abort.is($)) return;
    //
    //         let message = pb.collectAllErrorsAsFormattedList($);
    //         message ||= formatMessage({ defaultMessage: 'Failed to save SecondsInStatusbar' });
    //         toast.error(message, 'general-set-seconds-in-statusbar');
    //     } finally {
    //         await this.#fetchSystemInfo();
    //         this.#generalSetField('showSecondsStatusBar', s => getEmptyLeafState(s.value));
    //     }
    // };

    private generalSetDateFormatAbort = pb.abort.get();
    #generalSetDateFormat = async (value: pb.DateFormat): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('dateFormat', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetDateFormatAbort.replace();
            await pb.rpc.config.setDateFormat({ dateFormat: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Date format changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= formatMessage({ defaultMessage: 'Failed to save DateFormat' });
            toast.error(message);
        } finally {
            await this.#generalFetch();
            this.#setField('dateFormat', s => getFieldStateDefault(s.value));
        }
    };

    private generalSetFirsWeekDayAbort = pb.abort.get();
    #generalSetFirsWeekDay = async (value: pb.Weekday): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('firstDayOfWeek', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetFirsWeekDayAbort.replace();
            await pb.rpc.config.setFirstDayOfWeek({ firstDayOfWeek: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'First day of the week changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= formatMessage({ defaultMessage: 'Failed to save firs week day' });
            toast.error(message);
        } finally {
            await this.#generalFetch();
            this.#setField('firstDayOfWeek', s => getFieldStateDefault(s.value));
        }
    };

    private generalSetTemperatureUnitsAbort = pb.abort.get();
    #generalSetTemperatureUnits = async (value: pb.TemperatureUnit): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('temperatureUnit', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetTemperatureUnitsAbort.replace();
            await pb.rpc.config.setTemperatureUnit({ temperatureUnit: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Temperature units changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= formatMessage({ defaultMessage: 'Failed to save TemperatureUnits' });
            toast.error(message, { id: 'general-set-temperature-units' });
        } finally {
            await this.#generalFetch();
            this.#setField('temperatureUnit', s => getFieldStateDefault(s.value));
        }
    };

    private generalSetUnitSystemAbort = pb.abort.get();
    #generalSetUnitSystem = async (value: pb.UnitSystem): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('unitSystem', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetUnitSystemAbort.replace();
            await pb.rpc.config.setUnitSystem({ unitSystem: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Unit system changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= formatMessage({ defaultMessage: 'Failed to save unit system' });
            toast.error(message, { id: 'general-set-unit-system' });
        } finally {
            await this.#generalFetch();
            this.#setField('unitSystem', s => getFieldStateDefault(s.value));
        }
    };

    private generalSetNumberFormatAbort = pb.abort.get();
    #generalSetNumberFormat = async (value: pb.NumberFormat): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('numberFormat', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.generalSetNumberFormatAbort.replace();
            await pb.rpc.config.setNumberFormat({ numberFormat: value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Number format changed' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let message = pb.collectAllErrorsAsFormattedList($);
            message ||= formatMessage({ defaultMessage: 'Failed to save NumberFormat' });
            toast.error(message);
        } finally {
            await this.#generalFetch();
            this.#setField('numberFormat', s => getFieldStateDefault(s.value));
        }
    };

    // private generalSetDataCollectionAbort = pb.abort.get();
    // #generalSetDataCollection = async (value: boolean): Promise<void> => {
    //     const { formatMessage } = this.props.intl;
    //
    //     try {
    //         // Optimistic update & saving flag
    //         this.#setField('dataCollection', s => ({ ...s, value, isSaving: true }));
    //
    //         // Submit
    //         const { signal } = this.generalSetDataCollectionAbort.replace();
    //         await pb.rpc.config.setDataCollection({ value }, { signal });
    //
    //         toast.success(
    //             value
    //                 ? formatMessage({ defaultMessage: 'Data collection enabled' })
    //                 : formatMessage({ defaultMessage: 'Data collection disabled' }),
    //         );
    //     } catch ($) {
    //         if (pb.abort.is($)) return;
    //
    //         let message = pb.collectAllErrorsAsFormattedList($);
    //         message ||= formatMessage({ defaultMessage: 'Failed to save NumberFormat' });
    //         toast.error(message, 'general-set-number-format');
    //     } finally {
    //         await this.#generalFetch();
    //         this.#setField('dataCollection', s => getFieldStateDefault(s.value));
    //     }
    // };

    #generalFactoryReset = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            await pb.rpc.sys.factoryReset({});
            toast.success(formatMessage({ defaultMessage: 'Factory reset complete' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Unknown error!' });
            toast.error(msg);
        }
    };
    #generalSystemReboot = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            await pb.rpc.sys.reboot({});
            toast.success(formatMessage({ defaultMessage: 'System reboot triggered' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Unknown error!' });
            toast.error(msg);
        }
    };

    #generalDownloadSupportArchive = (): void => {
        downloadURL(URLS.api.supportArchive);
    };

    #generalRender = (): ReactNode => {
        const {
            data,
            values: {
                timeFormat,
                dateFormat,
                numberFormat,
                firstDayOfWeek,
                temperatureUnit,
                unitSystem,
                // showSecondsStatusBar,
                timezone,
                // dataCollection,
            },
        } = this.state;

        return (
            <SectionGeneral
                timeFormat={this.#getFieldStruct(timeFormat, this.#generalSetTimeFormat)}
                // secondsInStatusbar={this.#generalGetFieldStruct(showSecondsStatusBar, this.#generalSetSecondsInStatusbar)}

                // Regional
                timezone={this.#getFieldStruct(timezone, this.#generalSetTimezone, { items: data.timezones })}
                dateFormat={this.#getFieldStruct(dateFormat, this.#generalSetDateFormat)}
                firstWeekDay={this.#getFieldStruct(firstDayOfWeek, this.#generalSetFirsWeekDay)}
                temperatureUnits={this.#getFieldStruct(temperatureUnit, this.#generalSetTemperatureUnits)}
                unitSystem={this.#getFieldStruct(unitSystem, this.#generalSetUnitSystem)}
                numberFormat={this.#getFieldStruct(numberFormat, this.#generalSetNumberFormat)}
                // System actions
                onFactoryReset={this.#generalFactoryReset}
                onSystemReboot={this.#generalSystemReboot}
                onDownloadSupportArchive={this.#generalDownloadSupportArchive}
                // Usage data
                // usageData={this.#getFieldStruct(dataCollection, this.#generalSetDataCollection)}
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
                    displayNightmodeIntervalBackendValue: d.nightmodeInterval ?? null,
                } satisfies State['data'],
                values: {
                    ...s.values,
                    displayBrightness: getFieldStateDefault(d.brightness),
                    displayNightmodeEnabled: getFieldStateDefault(d.nightmodeEnabled),
                    displayNightmodeBrightness: getFieldStateDefault(d.brightnessNightmode),
                    displayNightmodeInterval: getFieldStateDefault(d.nightmodeInterval),
                } satisfies State['values'],
            }));
        } catch ($) {
            if (pb.abort.is($)) return;
            const msg: string = formatMessage({ defaultMessage: 'Failed to load display settings!' });
            toast.error(msg);
        }
    };

    private displaySetBrightnessAbort = pb.abort.get();
    #displaySetBrightness = debounce(async (value: Maybe<pb.BrightnessInfo>): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { displayBrightness } = this.state.values;

        if (value == null || value.value === displayBrightness.value?.value) return;

        try {
            // Optimistic update & saving flag
            this.#setField('displayBrightness', s => ({ ...s, value, isSaving: true }));

            // Submit
            const { signal } = this.displaySetBrightnessAbort.replace();
            await pb.rpc.config.setBrightness({ value: value.value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Brightness Saved' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors = pb.collectAllErrors($) ?? [
                formatMessage({ defaultMessage: 'Failed to save the brightness!' }),
            ];
            this.#setField('displayBrightness', s => ({ ...getFieldStateDefault(s.value), errors }));
        } finally {
            await this.#displayFetch();
            this.#setField('displayBrightness', s => getFieldStateDefault(s.value));
        }
    }, 600);

    private displaySetNightmodeEnabledAbort = pb.abort.get();
    #displaySetNightmodeEnabled = async (newEnabled: Maybe<boolean>): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { displayNightmodeEnabled, displayNightmodeInterval } = this.state.values;

        // Сurrent state snapshot
        const currentEnabled: null | boolean = displayNightmodeEnabled.value ?? null;
        const currentInterval: null | pb.TimeInterval = displayNightmodeInterval.value;
        const hasInterval: boolean = currentInterval != null;

        // Ignore redundant updates
        if (newEnabled == null || newEnabled === currentEnabled) return;

        try {
            // Optimistic update & saving flag
            await this.#setField('displayNightmodeEnabled', s => ({ ...s, value: newEnabled, isSaving: true }));

            // Persist changes
            const { signal } = this.displaySetNightmodeEnabledAbort.replace();
            const reqOpts = { signal };

            // Case 1: Disable nightmode or reuse existing interval (only update enabled flag)
            if (newEnabled === false || hasInterval) {
                await pb.rpc.config.setNightmodeEnabled({ value: newEnabled }, reqOpts);
            }

            // Case 2: First-time enable without interval
            // (enable nightmode AND initialize default interval)
            else {
                await Promise.all([
                    pb.rpc.config.setNightmodeEnabled({ value: newEnabled }, reqOpts),
                    pb.rpc.config.setNightmodeInterval(defaultNightmodeInterval, reqOpts),
                ]);
            }

            toast.success(formatMessage({ defaultMessage: 'Night Mode Saved' }));
        } catch ($) {
            // Error handling
            if (pb.abort.is($)) return;
            const errors = pb.collectAllErrors($) ?? [
                formatMessage({ defaultMessage: 'Failed to save the Night Mode!' }),
            ];
            this.#setFieldAttr('displayNightmodeEnabled', 'errors', errors);
        } finally {
            await this.#displayFetch();
            this.#setField('displayNightmodeEnabled', s => getFieldStateDefault(s.value));
        }
    };

    private displaySetNightmodeBrightnessAbort = pb.abort.get();
    #displaySetNightmodeBrightness = debounce(async (value: Maybe<pb.BrightnessInfo>): Promise<void> => {
        const { formatMessage } = this.props.intl;

        // Ignore redundant updates
        const { displayNightmodeBrightness } = this.state.values;
        const currentValue: null | number = displayNightmodeBrightness.value?.value ?? null;
        if (value == null || value.value === currentValue) return;

        try {
            // Optimistic update & saving flag
            await this.#setField('displayNightmodeBrightness', s => ({ ...s, value, isSaving: true }));

            // Persist changes
            const { signal } = this.displaySetNightmodeBrightnessAbort.replace();
            await pb.rpc.config.setBrightnessNightmode({ value: value.value }, { signal });

            toast.success(formatMessage({ defaultMessage: 'Night mode brightness saved' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors: string[] = pb.collectAllErrors($) ?? [
                formatMessage({ defaultMessage: 'Failed to save the night mode brightness!' }),
            ];
            this.#setFieldAttr('displayNightmodeBrightness', 'errors', errors);
        } finally {
            await this.#displayFetch();
            this.#setField('displayNightmodeBrightness', s => getFieldStateDefault(s.value));
        }
    }, 600);

    private displaySetNightmodeIntervalAbort = pb.abort.get();
    #displaySetNightmodeInterval = (value: pb.TimeInterval): void => {
        this.#setFieldAttr('displayNightmodeInterval', 'value', value);
    };
    #displaySubmitNightmodeInterval = debounce(async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        const value = this.state.values.displayNightmodeInterval.value;
        const validationErrors: string[] = [];

        // Validate time interval
        if (!value?.from || !value?.to)
            validationErrors.push(formatMessage({ defaultMessage: 'Time interval must be set!' }));
        else if (value.from === value.to)
            validationErrors.push(formatMessage({ defaultMessage: 'Start and end time cannot be the same!' }));
        else if (!validateTime(value.from) || !validateTime(value.to))
            validationErrors.push(formatMessage({ defaultMessage: 'Time must be in HH:MM format!' }));

        // Abort if we have errors
        if (validationErrors.length > 0)
            return this.#setFieldAttr('displayNightmodeInterval', 'errors', validationErrors);

        try {
            await this.#setFieldAttr('displayNightmodeInterval', 'isSaving', true);

            // Persist changes
            const { signal } = this.displaySetNightmodeIntervalAbort.replace();
            await pb.rpc.config.setNightmodeInterval(value as pb.TimeInterval, { signal });

            toast.success(formatMessage({ defaultMessage: 'Night mode time interval saved' }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors: string[] = pb.collectAllErrors($) ?? [
                formatMessage({ defaultMessage: 'Failed to save the night mode time interval!' }),
            ];
            this.#setFieldAttr('displayNightmodeInterval', 'errors', errors);
        } finally {
            await this.#displayFetch();
            this.#setField('displayNightmodeInterval', s => getFieldStateDefault(s.value));
        }
    }, 800);

    #displayRender = (): ReactNode => {
        const {
            data: { displayNightmodeIntervalBackendValue },
            values: {
                displayBrightness,
                displayNightmodeBrightness,
                displayNightmodeEnabled,
                displayNightmodeInterval,
            },
        } = this.state;

        return (
            <SectionDisplay
                brightness={this.#getFieldStruct(displayBrightness, this.#displaySetBrightness)}
                // Night
                nightEnabled={this.#getFieldStruct(displayNightmodeEnabled, this.#displaySetNightmodeEnabled)}
                nightBrightness={this.#getFieldStruct(displayNightmodeBrightness, this.#displaySetNightmodeBrightness)}
                nightInterval={this.#getFieldStruct(displayNightmodeInterval, this.#displaySetNightmodeInterval, {
                    hasChanged: !isEqual(displayNightmodeIntervalBackendValue, displayNightmodeInterval.value),
                    onConfirm: this.#displaySubmitNightmodeInterval,
                })}
                nightNotify={{
                    value: true,
                    disabled: true,
                    onChange: noop,
                }}
                // Location
                nightUseLocation={{
                    value: true,
                    disabled: true,
                    onChange: noop,
                }}
                nightLocation={{
                    value: 'Prague, Czechia',
                    disabled: true,
                    onChange: noop,
                }}
                onLocationDetect={noop}
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
            const [soundAndLight, ledSettings, bootSoundSettings] = await Promise.all([
                pb.rpc.config.getSoundVolumeSettings({}, { signal }),
                pb.rpc.config.getLedSettings({}, { signal }),
                pb.rpc.config.getBootSoundSettings({}, { signal }),
            ]);
            this.setState(s => ({
                values: {
                    ...s.values,
                    volume: getFieldStateDefault(soundAndLight.volume),
                    volumeNightmode: getFieldStateDefault(soundAndLight.volumeNightmode),
                    enableLedNotifications: getFieldStateDefault(ledSettings.ledEnabled),
                    enableLedNotificationsNightmode: getFieldStateDefault(ledSettings.ledEnabledNightmode),
                    enableBootSound: getFieldStateDefault(bootSoundSettings.bootSoundEnabled),
                },
            }));
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load sound settings!' });
            toast.error(msg);
        }
    };

    #soundLightSetVolume = debounce(async (value: pb.SoundVolume): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { volume } = this.state.values;

        // NOOP, required because the sliders are otherwise glitching this
        if (value.value === volume.value?.value) return;

        try {
            // Optimistic update & saving flag
            this.#setField('volume', s => ({ ...s, value, isSaving: true }));

            // Submit
            await pb.rpc.config.setSoundVolume({ value: value.value });

            toast.success(formatMessage({ defaultMessage: 'Sound volume saved' }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save the sound volume!' });
            toast.error(msg);
        } finally {
            await this.#soundLightFetch();
            this.#setField('volume', s => getFieldStateDefault(s.value));
        }
    }, 200);
    #soundLightSetVolumeNight = debounce(async (value: pb.SoundVolume): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { volumeNightmode } = this.state.values;

        // NOOP, required because the sliders are otherwise glitching this
        if (value.value === volumeNightmode.value?.value) return;

        try {
            // Optimistic update & saving flag
            this.#setField('volumeNightmode', s => ({ ...s, value, isSaving: true }));

            // Submit
            await pb.rpc.config.setSoundVolumeNightmode({ value: value.value });

            toast.success(formatMessage({ defaultMessage: 'Sound volume in night mode saved' }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save the sound volume in night mode!' });
            toast.error(msg);
        } finally {
            await this.#soundLightFetch();
            this.#setField('volumeNightmode', s => getFieldStateDefault(s.value));
        }
    }, 200);
    #soundLightSetBootSound = async (value: boolean): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('enableBootSound', s => ({ ...s, value, isSaving: true }));

            // Submit
            await pb.rpc.config.setBootSoundEnabled({ value });

            toast.success(
                value
                    ? formatMessage({ defaultMessage: 'Boot sound enabled' })
                    : formatMessage({ defaultMessage: 'Boot sound disabled' }),
            );
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save boot sound setting!' });
            toast.error(msg);
        } finally {
            await this.#soundLightFetch();
            this.#setField('enableBootSound', s => getFieldStateDefault(s.value));
        }
    };
    #soundLightSetLedNotify = async (value: boolean): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('enableLedNotifications', s => ({ ...s, value, isSaving: true }));

            // Submit
            await pb.rpc.config.setLedEnabled({ value });

            toast.success(
                value
                    ? formatMessage({ defaultMessage: 'LED notifications enabled' })
                    : formatMessage({ defaultMessage: 'LED notifications disabled' }),
            );
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save LED notifications setting!' });
            toast.error(msg);
        } finally {
            this.#soundLightFetch();
            this.#setField('enableLedNotifications', s => getFieldStateDefault(s.value));
        }
    };
    #soundLightSetLedNotifyNight = async (value: boolean): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // Optimistic update & saving flag
            this.#setField('enableLedNotificationsNightmode', s => ({ ...s, value, isSaving: true }));

            // Submit
            await pb.rpc.config.setLedEnabledNightmode({ value });

            toast.success(
                value
                    ? formatMessage({ defaultMessage: 'LED notifications in night mode enabled' })
                    : formatMessage({ defaultMessage: 'LED notifications in night mode disabled' }),
            );
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to save LED notifications night mode setting!' });
            toast.error(msg);
        } finally {
            this.#soundLightFetch();
            this.#setField('enableLedNotificationsNightmode', s => getFieldStateDefault(s.value));
        }
    };
    #soundLightRender = (): ReactNode => {
        const { volume, volumeNightmode, enableLedNotifications } = this.state.values;
        const { enableLedNotificationsNightmode, enableBootSound } = this.state.values;

        return (
            <SectionSoundAndLight
                soundVolume={this.#getFieldStruct<pb.SoundVolume>(volume, this.#soundLightSetVolume)}
                soundVolumeNight={this.#getFieldStruct<pb.SoundVolume>(volumeNightmode, this.#soundLightSetVolumeNight)}
                // alarmAndNotifyVolume={{ value: 65, onChange: noop }}
                bootSoundEnabled={this.#getFieldStruct<boolean>(enableBootSound, this.#soundLightSetBootSound)}
                ledNotifyEnabled={this.#getFieldStruct<boolean>(enableLedNotifications, this.#soundLightSetLedNotify)}
                ledNotifyEnabledNight={this.#getFieldStruct<boolean>(
                    enableLedNotificationsNightmode,
                    this.#soundLightSetLedNotifyNight,
                )}
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
        return <SectionSecurity hasPassword={hasPassword} actions={this.#secActions} />;
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
            toast.error(message);

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
            toast.error(message);
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
        const { upgradeFromFeedStatus, upgradeFromFeedErrors, data } = this.state;
        return (
            <SectionUpgrade
                automaticUpgrades={{ value: true, onChange: this.#updatesToggle, disabled: true }}
                versionCurrent={data.metadata?.version ?? null}
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
