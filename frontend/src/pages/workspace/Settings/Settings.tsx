import { Component } from 'react';

import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { type IntlShape, useIntl } from 'react-intl';
import { type Location, useLocation } from 'react-router';

import { assertUnreachable } from '@/lib/ts';
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
        upgradeInfo: null | pb.CheckForUpgradeResponse;
    };

    genTimezone: LeafState<pb.Timezone>;

    secAllowDataCollection: LeafState<boolean>;
}
const getInitialState = (): State => ({
    activeTab: Tab.general,
    globalErrors: null,
    data: {
        timezones: [],
        upgradeInfo: null,
    },

    genTimezone: getEmptyLeafState(),
    secAllowDataCollection: getEmptyLeafState(),
});

class View extends Component<Props, State> {
    readonly state = getInitialState();

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
        else if (Object.hasOwn(Tab, maybeTabHash)) this.#tabChange(maybeTabHash as Tab);
    };

    #noop = () => {};
    #fetchData = async (): Promise<void> => {
        await Promise.allSettled([this.#fetchUpgradeInfo(), this.#fetchSystemInfo()]);
    };

    private fetchUpgradeInfoAbort = pb.abort.get();
    #fetchUpgradeInfo = async (): Promise<void> => {
        try {
            const { signal } = this.fetchUpgradeInfoAbort.replace();
            const upgradeInfo = await pb.rpc.upgrade.checkForUpgrade({}, { signal });
            this.setState(s => ({ data: { ...s.data, upgradeInfo } }));
        } catch ($) {
            if (pb.abort.is($)) return;
            const err = pb.parseError($);
            const errors = pb.parseFormErrors(err, []);
            this.setState({ globalErrors: errors.global });
        }
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
            const err = pb.parseError($);
            const errors = pb.parseFormErrors(err, []);
            this.setState({ globalErrors: errors.global });
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
        this.setState({ activeTab: tab });
        window.history.replaceState(null, '', `#${tab}`);
    };

    //
    // General
    //

    private generalSetTimezoneAbort = pb.abort.get();
    #generalSetTimezone = async (value: pb.Timezone): Promise<void> => {
        try {
            const { signal } = this.generalSetTimezoneAbort.replace();
            await setState(this, s => ({ genTimezone: getEmptyLeafState(s.genTimezone.value, 'saving') }));
            await pb.rpc.sys.setTimezone({ id: value.id }, { signal });
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.parseFormErrors(pb.parseError($), []).global;
            this.setState(s => ({ genTimezone: { ...s.genTimezone, errors } }));
        } finally {
            await this.#fetchSystemInfo();
            this.setState(s => ({ genTimezone: getEmptyLeafState(s.genTimezone.value) }));
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
                onFactoryReset={undefined}
            />
        );
    };

    //
    // Display
    //

    #displayRender = (): ReactNode => {
        return (
            <SectionDisplay
                brightnessDay={{
                    value: 78,
                    disabled: true,
                    onChange: this.#noop,
                }}
                // Night
                nightBrightness={{
                    value: 26,
                    disabled: true,
                    onChange: this.#noop,
                }}
                nightEnabled={{
                    value: true,
                    disabled: true,
                    onChange: this.#noop,
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

    #soundLightRender = (): ReactNode => {
        return (
            <SectionSoundAndLight
                soundVolume={{
                    value: 84,
                    disabled: true,
                    onChange: this.#noop,
                }}
                soundVolumeNight={{
                    value: 21,
                    disabled: true,
                    onChange: this.#noop,
                }}
                alarmAndNotifyVolume={{
                    value: 65,
                    disabled: true,
                    onChange: this.#noop,
                }}
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

    #updatesToggle = (enabled: boolean): void => console.log(enabled);
    #updatesRender = (): ReactNode => {
        const { upgradeInfo } = this.state.data;
        return (
            <SectionUpgrade
                automaticUpgrades={{
                    value: true,
                    onChange: this.#updatesToggle,
                    disabled: true,
                }}
                versionCurrent="24.04.1"
                upgradeInfo={upgradeInfo}
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
            <div>
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
