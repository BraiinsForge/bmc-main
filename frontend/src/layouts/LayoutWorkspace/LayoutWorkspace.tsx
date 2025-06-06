import { Fragment, Component, useCallback, type UIEvent, type KeyboardEvent } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { useLocation, useNavigate } from 'react-router';
import { Key } from 'ts-key-enum';

import { URLS } from '@/constants';
import { store, useStore } from '@/store';
import { Braiins, Bitcoin } from '@/components';

import {
    Content,
    Header,
    HeaderContainer,
    HeaderName,
    HeaderMenuButton,
    HeaderGlobalBar,
    HeaderGlobalAction,
    SkipToContent,
    SideNav,
    SideNavItems,
    SideNavLink,
} from '@carbon/react';
import {
    type CarbonIconType,
    NotebookReference,
    Logout as IconLogout,
    Settings as IconSettings,
    Tag as IconPriceAlerts,
    Notification as IconNotification,
    Screen as IconScreen,
    Alarm as IconAlarm,
    Network_2 as IconNetwork,
    Api_1 as IconApi,
} from '@carbon/icons-react';

import css from './LayoutWorkspace.scss';

export interface LayoutWorkspaceProps {
    children: ReactNode;
}
interface Props extends LayoutWorkspaceProps {
    intl: IntlShape;
    hasPassword: null | boolean;
}

interface State {}
const getInitialState = (): State => ({});

class Base extends Component<Props, State> {
    readonly state = getInitialState();
    #txt = {
        name: this.props.intl.formatMessage({ defaultMessage: 'BMC 100 - Braiins Mining Clock' }),
        sidenav: this.props.intl.formatMessage({ defaultMessage: 'Side navigation' }),

        documentation: this.props.intl.formatMessage({ defaultMessage: 'Documentation' }),
        logout: this.props.intl.formatMessage({ defaultMessage: 'Logout' }),
    };

    #renderSidenavItems = (): ReactNode => {
        const { formatMessage } = this.props.intl;

        return (
            <Fragment>
                <SideLink
                    icon={IconScreen}
                    url={URLS.pages.display}
                    label={formatMessage({ defaultMessage: 'Display Scenes' })}
                />
                <SideLink
                    icon={IconSettings}
                    url={URLS.pages.settings}
                    label={formatMessage({ defaultMessage: 'System Settings' })}
                />
                <SideLink
                    icon={IconNetwork}
                    url={URLS.pages.network}
                    label={formatMessage({ defaultMessage: 'Network Configuration' })}
                />
                <SideLink
                    icon={IconAlarm}
                    url={URLS.pages.alarms}
                    label={formatMessage({ defaultMessage: 'Alarms' })}
                />
                <SideLink
                    icon={IconPriceAlerts}
                    url={URLS.pages.priceAlerts}
                    label={formatMessage({ defaultMessage: 'Price Alerts' })}
                />
                <SideLink
                    icon={IconNotification}
                    url={URLS.pages.notifications}
                    label={formatMessage({ defaultMessage: 'Notifications' })}
                />
                <SideLink
                    icon={IconApi}
                    url={URLS.pages.api}
                    label={formatMessage({ defaultMessage: 'API Connections' })}
                />
                <SideLink
                    icon={Bitcoin}
                    url={URLS.pages.buyButton}
                    label={formatMessage({ defaultMessage: 'Braiins Buy Button' })}
                />
            </Fragment>
        );
    };

    #renderContent = (x: { isSideNavExpanded: boolean; onClickSideNavExpand(): void }): ReactNode => {
        const { isSideNavExpanded, onClickSideNavExpand } = x;
        const { children, hasPassword } = this.props;

        return (
            <Fragment>
                <Header aria-label={this.#txt.name}>
                    <SkipToContent />

                    <HeaderMenuButton
                        aria-label={isSideNavExpanded ? 'Close menu' : 'Open menu'}
                        onClick={onClickSideNavExpand}
                        isActive={isSideNavExpanded}
                        aria-expanded={isSideNavExpanded}
                    />

                    <HeaderName href={URLS.pages.settings} prefix="" className={css.headerName}>
                        <Braiins width={114} />
                        <span className={css.headerNameText} children={this.#txt.name} />
                    </HeaderName>

                    <HeaderGlobalBar>
                        <HeaderActionButton
                            label={this.#txt.documentation}
                            icon={NotebookReference}
                            onClick={() => window.open(URLS.external.academy, '_blank')}
                            withInlineLabel
                        />
                        {hasPassword && (
                            <HeaderActionButton label={this.#txt.logout} icon={IconLogout} onClick={store.logout} />
                        )}
                    </HeaderGlobalBar>
                </Header>

                <SideNav
                    expanded={isSideNavExpanded}
                    aria-label={this.#txt.sidenav}
                    className={css.sidenav}
                    onToggle={onClickSideNavExpand}
                >
                    <SideNavItems children={this.#renderSidenavItems()} />
                </SideNav>

                <Content id="main-content" className={css.main} children={children} />
            </Fragment>
        );
    };

    render() {
        return <HeaderContainer render={this.#renderContent} />;
    }
}

export function LayoutWorkspace(props: LayoutWorkspaceProps) {
    const intl = useIntl();
    const hasPassword: null | boolean = useStore(x => x.state.sessionInfo.hasPassword);
    return <Base {...props} intl={intl} hasPassword={hasPassword} />;
}

interface HeaderActionButtonProps {
    onClick(): void;
    label: string;
    icon: CarbonIconType;
    withInlineLabel?: boolean;
    tooltipAlignment?: 'start' | 'center' | 'end';
}
function HeaderActionButton(props: HeaderActionButtonProps) {
    const { onClick, label, icon: Icon, withInlineLabel, tooltipAlignment } = props;

    return (
        <HeaderGlobalAction aria-label={label} onClick={onClick} tooltipAlignment={tooltipAlignment ?? 'end'}>
            <div className={css.headerAction}>
                <Icon size={20} />
                {withInlineLabel ? <span className={css.label} children={label} /> : null}
            </div>
        </HeaderGlobalAction>
    );
}

interface SideLink {
    icon: CarbonIconType;
    label: string;
    url: string;
}
function SideLink(props: SideLink) {
    const { label, icon: Icon, url } = props;

    const location = useLocation();
    const navigate = useNavigate();

    const isActive: boolean = location.pathname.startsWith(url);
    const handleClick = useCallback(
        (e: UIEvent) => {
            if (e.type === 'click') {
                navigate(url);
                (document.activeElement as Maybe<HTMLElement>)?.blur();
            } else if (e.type === 'keydown') {
                const key = (e as KeyboardEvent).key;
                if (key === Key.Enter || key === ' ') navigate(url);
            }
        },
        [url, navigate],
    );

    return (
        <SideNavLink
            className={css.sideLink}
            renderIcon={Icon}
            onClick={handleClick}
            onKeyDown={handleClick}
            children={label}
            isActive={isActive}
        />
    );
}
