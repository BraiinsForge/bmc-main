// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { Fragment, Component, useCallback, type UIEvent, type KeyboardEvent, type MouseEvent } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { useLocation, useNavigate, type NavigateFunction } from 'react-router';
import { Key } from 'ts-key-enum';

import { URLS } from '@/constants';
import { store, useStore } from '@/store';
import type { Capabilities } from '@/lib/system';
import { alarmsAvailable, boserChrome, networkConfigurable } from '@/lib/capabilities';
import type { Brand } from '@/lib/brand';
import { ARIA, LogoHeader, LinksBar } from '@/components';
import cn from 'clsx';

import {
    Content,
    Header,
    HeaderContainer,
    HeaderName,
    HeaderMenuButton,
    HeaderGlobalBar,
    HeaderGlobalAction,
    HeaderNavigation,
    HeaderMenuItem,
    SkipToContent,
    SideNav,
    SideNavItems,
    SideNavLink,
} from '@carbon/react';
import {
    type CarbonIconType,
    NotebookReference,
    Logout as IconLogout,
    Screen as IconScreen,
    Settings as IconSettings,
    Network_2 as IconNetwork,
    Alarm as IconAlarm,
    Api_1 as IconApi,
    // Tag as IconPriceAlerts,
    // Notification as IconNotification,
} from '@carbon/icons-react';

import css from './LayoutWorkspace.scss';

export interface LayoutWorkspaceProps {
    children: ReactNode;
}
interface Props extends LayoutWorkspaceProps {
    intl: IntlShape;
    navigate: NavigateFunction;
    hasPassword: null | boolean;
    capabilities: Capabilities;
    brand: null | Brand;
}

interface State {}
const getInitialState = (): State => ({});

class Base extends Component<Props, State> {
    readonly state = getInitialState();
    #txt = {
        name: this.props.intl.formatMessage({ defaultMessage: 'Braiins DECK' }),
        sidenav: this.props.intl.formatMessage({ defaultMessage: 'Side navigation' }),

        topnav: this.props.intl.formatMessage({ defaultMessage: 'Top level navigation' }),
        dashboard: this.props.intl.formatMessage({ defaultMessage: 'Dashboard' }),
        configuration: this.props.intl.formatMessage({ defaultMessage: 'Configuration' }),
        system: this.props.intl.formatMessage({ defaultMessage: 'System' }),
        display: this.props.intl.formatMessage({ defaultMessage: 'Display' }),
        documentation: this.props.intl.formatMessage({ defaultMessage: 'Documentation' }),
        logout: this.props.intl.formatMessage({ defaultMessage: 'Logout' }),
    };

    #gotHome = (): void => {
        this.props.navigate(URLS.pages.display.list);
    };
    #gotoDisplay = (event: MouseEvent<HTMLAnchorElement>): void => {
        event.preventDefault();
        this.#gotHome();
    };
    // Below Carbon's lg breakpoint the header tabs are gone,
    // so like boser the drawer carries them as one section per tab; CSS hides them above it.
    #renderBoserSections = (): ReactNode => (
        <div className={css.boserSections}>
            <SideSection title={this.#txt.dashboard} />
            <SideNavLink href={URLS.boser.dashboard} className={css.sideLink} children={this.#txt.dashboard} />
            <SideSection title={this.#txt.configuration} />
            <SideNavLink href={URLS.boser.configuration} className={css.sideLink} children={this.#txt.configuration} />
            <SideSection title={this.#txt.system} />
            <SideNavLink href={URLS.boser.system} className={css.sideLink} children={this.#txt.system} />
            <SideSection title={this.#txt.display} />
        </div>
    );
    #renderSidenavItems = (): ReactNode => {
        const { formatMessage } = this.props.intl;

        return (
            <Fragment>
                {boserChrome(this.props.capabilities) && this.#renderBoserSections()}
                <SideLink
                    icon={IconScreen}
                    url={URLS.pages.display.list}
                    label={formatMessage({ defaultMessage: 'Display Widgets' })}
                />
                <SideLink
                    icon={IconSettings}
                    url={URLS.pages.settings}
                    label={formatMessage({ defaultMessage: 'System Settings' })}
                />
                {networkConfigurable(this.props.capabilities) && (
                    <SideLink
                        icon={IconNetwork}
                        url={URLS.pages.network}
                        label={formatMessage({ defaultMessage: 'Network Configuration' })}
                    />
                )}
                <SideLink
                    icon={IconApi}
                    url={URLS.pages.accounts}
                    label={formatMessage({ defaultMessage: 'Connected Accounts' })}
                />
                {alarmsAvailable(this.props.capabilities) && (
                    <SideLink
                        icon={IconAlarm}
                        url={URLS.pages.alarms}
                        label={formatMessage({ defaultMessage: 'Alarms' })}
                    />
                )}
                {/*
                <SideLink icon={IconPriceAlerts} url={URLS.pages.priceAlerts} label={formatMessage({ defaultMessage: 'Price Alerts' })} />
                <SideLink icon={IconNotification} url={URLS.pages.notifications} label={formatMessage({ defaultMessage: 'Notifications' })} />
                */}
            </Fragment>
        );
    };

    #renderContent = (x: { isSideNavExpanded: boolean; onClickSideNavExpand(): void }): ReactNode => {
        const { isSideNavExpanded, onClickSideNavExpand } = x;
        const { children, hasPassword, capabilities, brand } = this.props;
        const boserUi = boserChrome(capabilities);
        const links = brand?.links.products ?? [];
        const logo = brand?.logo.header ?? null;

        // The boser classes need a carrier and a Fragment cannot take one;
        // Carbon positions the header and side nav `fixed`, so the div itself has no layout effect.
        return (
            <div className={cn(boserUi && css.boserLayout, boserUi && links.length > 0 && css.withLinksBar)}>
                {boserUi && <LinksBar links={links} className={css.linksBar} />}
                <Header aria-label={brand?.name ?? this.#txt.name}>
                    <SkipToContent />

                    <HeaderMenuButton
                        aria-label={isSideNavExpanded ? 'Close menu' : 'Open menu'}
                        onClick={onClickSideNavExpand}
                        isActive={isSideNavExpanded}
                        aria-expanded={isSideNavExpanded}
                    />

                    {boserUi && logo && brand ? (
                        <div className={css.headerNameBoser} {...ARIA.button(this.#gotHome)}>
                            <img
                                src={logo.src}
                                style={logo.style}
                                alt={`${brand.name ?? 'Braiins OS'} logo`}
                                className={css.brandLogo}
                            />
                        </div>
                    ) : (
                        <HeaderName prefix="" onClick={this.#gotHome} className={css.headerName}>
                            <LogoHeader style={{ width: 'auto', height: 18 }} />
                        </HeaderName>
                    )}

                    {boserUi && (
                        <HeaderNavigation aria-label={this.#txt.topnav}>
                            <HeaderMenuItem href={URLS.boser.dashboard} children={this.#txt.dashboard} />
                            <HeaderMenuItem href={URLS.boser.configuration} children={this.#txt.configuration} />
                            <HeaderMenuItem href={URLS.boser.system} children={this.#txt.system} />
                            <HeaderMenuItem
                                href={URLS.pages.display.list}
                                onClick={this.#gotoDisplay}
                                aria-current="page"
                                children={this.#txt.display}
                            />
                        </HeaderNavigation>
                    )}

                    <HeaderGlobalBar>
                        <HeaderActionButton
                            label={this.#txt.documentation}
                            icon={NotebookReference}
                            onClick={() => window.open(URLS.external.academy, '_blank')}
                            withInlineLabel
                        />
                        {/* Next to boser the login is boser's, and it shows its logout even without a password. */}
                        {boserUi || hasPassword ? (
                            <HeaderActionButton
                                label={this.#txt.logout}
                                icon={IconLogout}
                                onClick={boserUi ? store.logoutToBoser : store.logout}
                            />
                        ) : null}
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
            </div>
        );
    };

    render() {
        return <HeaderContainer render={this.#renderContent} />;
    }
}

export function LayoutWorkspace(props: LayoutWorkspaceProps) {
    const intl = useIntl();
    const hasPassword: null | boolean = useStore(x => x.state.sessionInfo.hasPassword);
    const capabilities = useStore(x => x.state.hardwareCapabilities);
    const brand = useStore(x => x.state.brand);
    const navigate = useNavigate();
    return (
        <Base
            {...props}
            intl={intl}
            navigate={navigate}
            hasPassword={hasPassword}
            capabilities={capabilities}
            brand={brand}
        />
    );
}

function SideSection(props: { title: string }) {
    return <div className={css.sideSection} role="heading" aria-level={2} children={props.title} />;
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
