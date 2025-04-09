import { Component } from 'react';

import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { useIntl, type IntlShape } from 'react-intl';
import { useLocation, type Location } from 'react-router';

import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

import { SectionSecurity, SectionUpgrade } from './components';
import { InlineNotificationsGroup, Tabs, type TabsProps } from '@/components';
import css from './Settings.scss';

interface Props {
    intl: IntlShape;
    location: Location;
}

// The value is used in location hash, so be mindfull of that.
enum Tab {
    general = 'general',
    display = 'display',
    soundAndLight = 'sound-and-light',
    security = 'security',
    updates = 'updates',
}
interface State {
    activeTab: Tab;
    globalErrors: Maybe<string[]>;

    allowDataCollection: boolean;
    hasPassword: null | boolean;

    upgradeInfo: null | pb.CheckForUpgradeResponse;
}
const getInitialState = (): State => ({
    activeTab: Tab.general,
    globalErrors: null,

    allowDataCollection: true,
    hasPassword: null,

    upgradeInfo: null,
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

    #fetchData = async (): Promise<void> => {
        await Promise.all([this.#fetchPasswordInfo(), this.#fetchUpgradeInfo()]);
    };

    private fetchPasswordInfoAbort = pb.abort.get();
    #fetchPasswordInfo = async (): Promise<void> => {
        try {
            const { signal } = this.fetchPasswordInfoAbort.replace();
            const res = await pb.rpc.sys.hasPassword({}, { signal });
            this.setState({ hasPassword: res.value });
        } catch ($) {
            if (pb.abort.is($)) return;
            const err = pb.parseError($);
            const errors = pb.parseFormErrors(err, []);
            this.setState({ globalErrors: errors.global });
        }
    };

    private fetchUpgradeInfoAbort = pb.abort.get();
    #fetchUpgradeInfo = async (): Promise<void> => {
        try {
            const { signal } = this.fetchUpgradeInfoAbort.replace();
            const upgradeInfo = await pb.rpc.upgrade.checkForUpgrade({}, { signal });
            this.setState({ upgradeInfo });
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
    // Security
    //

    #secOnPasswordChange = async (data: pb.ChangePasswordRequest): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { hasPassword } = this.state;

        // Abort if we got called wrongly
        if (hasPassword !== true) {
            return this.setState({
                globalErrors: [formatMessage({ defaultMessage: 'Password is not set!' })],
            });
        }

        await pb.rpc.sys.changePassword(data);
        await this.#fetchPasswordInfo();
    };
    #secOnPasswordRemove = async (data: pb.RemovePasswordRequest): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { hasPassword } = this.state;

        // Abort if we got called wrongly
        if (hasPassword !== true) {
            return this.setState({
                globalErrors: [formatMessage({ defaultMessage: 'Password is not set!' })],
            });
        }

        await pb.rpc.sys.removePassword(data);
        await this.#fetchPasswordInfo();
    };
    #secOnPasswordCreate = async (data: pb.CreatePasswordRequest): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { hasPassword } = this.state;

        // Abort if we got called wrongly
        if (hasPassword !== false) {
            return this.setState({
                globalErrors: [formatMessage({ defaultMessage: 'Password is already set!' })],
            });
        }

        await pb.rpc.sys.createPassword(data);
        await this.#fetchPasswordInfo();
    };
    #secRender = (): ReactNode => {
        const { hasPassword, allowDataCollection } = this.state;
        return (
            <SectionSecurity
                hasPassword={hasPassword}
                onPasswordChange={this.#secOnPasswordChange}
                onPasswordRemove={this.#secOnPasswordRemove}
                onPasswordCreate={this.#secOnPasswordCreate}
                dataCollection={{
                    value: allowDataCollection,
                    disabled: true,
                    // FIXME: Implement the setter
                    onChange: async (): Promise<void> => {},
                }}
            />
        );
    };

    //
    // Upgrades
    //

    #updatesToggle = (enabled: boolean): void => console.log(enabled);
    #updatesRender = (): ReactNode => {
        const { upgradeInfo } = this.state;
        return (
            <SectionUpgrade
                automaticUpgrades={{ value: true, onChange: this.#updatesToggle, disabled: true }}
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
                content = null;
                break;

            case Tab.display:
                content = null;
                break;

            case Tab.soundAndLight:
                content = null;
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
                <InlineNotificationsGroup kind="error" items={globalErrors} stretch theme="inverse" />
                <div className={css.content} children={content} />
            </div>
        );
    }
}

export default function () {
    const intl = useIntl();
    const location = useLocation();
    return <View intl={intl} location={location} />;
}
