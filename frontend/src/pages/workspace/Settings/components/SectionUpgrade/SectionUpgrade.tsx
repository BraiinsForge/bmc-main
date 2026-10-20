// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { Component } from 'react';
import Markdown from 'markdown-it';
import { formatDuration } from 'date-fns';
import { FormattedMessage, useIntl, type IntlShape } from 'react-intl';

// App
import * as pb from '@/proto';

// Lib
import { getID } from '../../const';
import type { iField } from '@/lib/form';

// Components
import {
    Field,
    FieldSet,
    Button,
    InlineNotification,
    InlineNotificationsGroup,
    Html,
    Overlay,
    Tick,
} from '@/components';
import { InlineLoading, Toggle, ToggleSkeleton } from '@carbon/react';
import { Upgrade as IconUpgrade, ChevronDown, ChevronUp, Restart as IconRestart } from '@carbon/react/icons';

// Styles
import css from './SectionUpgrade.scss';
import { assertUnreachable } from '@/lib/ts';
import { getTimestamp } from '@/lib/time';
import { formatBytes } from '@/lib/format';
import { PackageChanges } from './PackageChanges';
import { UpgradeStepper, type UpgradeStepperProps } from './UpgradeStepper';

export type UpgradeFromFeedStatus =
    | {
          kind: 'idle';
          upgradeInfo: null;
      }
    | {
          kind: 'checking-upgrade';
          upgradeInfo: null;
      }
    | {
          kind: 'up-to-date';
          upgradeInfo: null;
      }
    | {
          kind: 'upgrade-available';
          upgradeInfo: pb.CheckForUpgradeResponse;
      }
    | {
          kind: 'upgrading';
          upgradeInfo: pb.CheckForUpgradeResponse;
          progress: UpgradeStepperProps;
      }
    | {
          kind: 'restarting';
          upgradeInfo: null;
          disruption: pb.UpgradeDisruption;
          startTime: Timestamp;
      };

export interface SectionUpgradeProps {
    automaticUpgrades: iField<boolean>;
    /** Firmware (BOS) version the device runs; `null` when unknown. */
    systemVersion: null | string;
    /** Packages version of the installed core; `null` when it carries none. */
    packagesVersion: null | string;

    status: null | UpgradeFromFeedStatus;
    errors?: Maybe<string[]>;

    onCheckUpdates(): void;
    onStartUpgrade(upgradeId: string): void;
}
interface Props extends SectionUpgradeProps {
    intl: IntlShape;
}

interface State {
    isChangelogExpanded: boolean;
}
const getInitialState = (): State => ({
    isChangelogExpanded: false,
});

const $ = getID('updates').get;

class View extends Component<Props, State> {
    #milis = {
        restartingTick: 1e3,
    };
    #NA = <span className={css.placeholder} children="N/A" />;

    readonly state = getInitialState();

    componentDidUpdate(prevProps: Props) {
        const { status } = this.props;
        if (
            status?.kind !== prevProps.status?.kind ||
            status?.upgradeInfo?.upgradeId !== prevProps.status?.upgradeInfo?.upgradeId
        ) {
            this.setState({ isChangelogExpanded: false });
        }
    }

    #md = Markdown('default', {
        html: false,
        breaks: false,
        linkify: true,
        typographer: false,
    });
    #renderDescription = (version: string, description: string): ReactElement => {
        return (
            <div key={version} className={css.description}>
                <h1 children={version} className={css.version} />
                <Html key={version} container="div" source={this.#md.render(description)} />
            </div>
        );
    };

    #toggleChangelog = () => this.setState(s => ({ isChangelogExpanded: !s.isChangelogExpanded }));
    #renderChangelog = (): null | ReactElement => {
        const { status, intl } = this.props;
        const { isChangelogExpanded } = this.state;

        const firmware = status?.upgradeInfo?.firmware;
        const packages = status?.upgradeInfo?.packages;

        // Firmware release notes headline the changelog;
        // A packages-only upgrade falls back to the bundle's bmc changelog.
        // Older-release history is firmware-only.
        const primary = firmware
            ? { version: firmware.version, description: firmware.description }
            : packages?.bmcChangelog
              ? { version: packages.bmcVersion ?? '', description: packages.bmcChangelog }
              : null;
        if (!primary) return null;

        const previousReleases = firmware?.previousReleases ?? [];

        let expandedContent: ReactNode;
        let expanderButton: ReactNode;
        if (previousReleases.length) {
            if (isChangelogExpanded)
                expandedContent = previousReleases.map(x => this.#renderDescription(x.version, x.description));

            expanderButton = (
                <Button
                    id={$('changelog-expander')}
                    kind="tertiary"
                    onClick={this.#toggleChangelog}
                    children={
                        isChangelogExpanded
                            ? intl.formatMessage({ defaultMessage: 'Hide Older Updates' })
                            : intl.formatMessage(
                                  {
                                      defaultMessage:
                                          '{count, plural, one {+{count} Older Update} other {+{count} Older Updates}}',
                                  },
                                  { count: previousReleases.length },
                              )
                    }
                    icon={isChangelogExpanded ? ChevronUp : ChevronDown}
                />
            );
        }

        return (
            <div className={css.changelog}>
                <h2
                    className={css.title}
                    children={intl.formatMessage({ defaultMessage: 'Whats new in this Upgrade?' })}
                />

                {this.#renderDescription(primary.version, primary.description)}

                {expandedContent}
                {expanderButton}
            </div>
        );
    };

    #renderInlineNotificationError = (error: string | string[], key: string): ReactElement => {
        return Array.isArray(error) ? (
            <InlineNotificationsGroup stretch key={key} kind="error" theme="inverse" items={error} />
        ) : (
            <InlineNotification stretch key={key} kind="error" theme="inverse" hideCloseButton title={error} />
        );
    };
    #renderFacts = (
        versions: {
            systemCurrent: Maybe<ReactNode>;
            systemLatest: Maybe<ReactNode>;
            packages: null | { current: Maybe<ReactNode>; latest: Maybe<ReactNode> };
        },
        statusMessage: ReactNode,
        downloadSize?: Maybe<ReactNode>,
    ) => {
        const { formatMessage } = this.props.intl;
        type Row = [string, Maybe<ReactNode>];
        const groups: Row[][] = [
            [
                [formatMessage({ defaultMessage: 'Your system version:' }), versions.systemCurrent],
                [formatMessage({ defaultMessage: 'Latest system version:' }), versions.systemLatest],
            ],
        ];
        if (versions.packages) {
            groups.push([
                [formatMessage({ defaultMessage: 'Your packages version:' }), versions.packages.current],
                [formatMessage({ defaultMessage: 'Latest packages version:' }), versions.packages.latest],
            ]);
        }
        if (downloadSize != null) {
            groups.push([[formatMessage({ defaultMessage: 'Download size:' }), downloadSize]]);
        }

        return (
            <div className={css.facts}>
                <table>
                    {groups.map(rows => (
                        <tbody key={rows[0][0]}>
                            {rows.map(([label, value]) => (
                                <tr key={label}>
                                    <th scope="row" children={label} />
                                    <td children={value ?? 'N/A'} />
                                </tr>
                            ))}
                        </tbody>
                    ))}
                </table>

                <div className={css.statusMessage} children={statusMessage} />
            </div>
        );
    };
    #renderInterval = (seconds: number): ReactNode => {
        return formatDuration({ seconds: Math.abs(seconds) }, { zero: true, format: ['minutes', 'seconds'] });
    };
    #renderUpgradeBox(): ReactNode {
        const {
            intl: { formatMessage },

            status,
            errors,
            systemVersion,
            packagesVersion,

            onCheckUpdates,
            onStartUpgrade,
        } = this.props;

        const systemLatest: ReactNode = status?.upgradeInfo?.firmware?.version ?? systemVersion ?? this.#NA;
        // A packages offer without a stamp removes the installed one, so the
        // installed stamp stands in for the latest only while nothing is offered.
        const packagesOffer = status?.upgradeInfo?.packages;
        const packagesLatest = packagesOffer ? packagesOffer.bmcVersion : packagesVersion;

        // Both latest cells show the spinner while a check runs.
        const whileChecking = (latest: ReactNode): ReactNode =>
            status?.kind === 'checking-upgrade' ? (
                <InlineLoading
                    status="active"
                    description={formatMessage({ defaultMessage: 'Checking for updates…' })}
                />
            ) : (
                latest
            );
        const versions = {
            systemCurrent: systemVersion,
            systemLatest: whileChecking(systemLatest),
            packages:
                packagesLatest != null || packagesVersion != null
                    ? { current: packagesVersion, latest: whileChecking(packagesLatest ?? this.#NA) }
                    : null,
        };
        const changelog = this.#renderChangelog();

        // Total bytes this upgrade fetches — firmware image plus package downloads —
        // surfaced on the offer so the size is known before committing.
        const offer = status?.kind === 'upgrade-available' ? status.upgradeInfo : null;
        const downloadBytes = (offer?.firmware?.fileSizeBytes ?? 0n) + (offer?.packages?.downloadSizeBytes ?? 0n);
        const downloadSize = downloadBytes > 0n ? formatBytes(downloadBytes) : null;

        let statusMessage: ReactNode;
        let control: ReactNode;
        let overlayContent: ReactNode;

        switch (status?.kind) {
            case undefined:
            case 'idle':
            case 'checking-upgrade':
                if (errors) statusMessage = this.#renderInlineNotificationError(errors, 'error-idle-or-checking');
                break;

            case 'up-to-date': {
                statusMessage = [
                    <InlineNotification
                        key="info"
                        stretch
                        kind="success"
                        theme="inverse"
                        hideCloseButton
                        title={formatMessage({ defaultMessage: 'You are up to date' })}
                        children={formatMessage({ defaultMessage: 'Your system and packages are up to date' })}
                    />,
                    errors?.length ? this.#renderInlineNotificationError(errors, 'error-up-to-date') : null,
                ];
                control = (
                    <Button
                        id={$('check-for-updates')}
                        kind="tertiary"
                        onClick={onCheckUpdates}
                        children={formatMessage({ defaultMessage: 'Check for a new version' })}
                    />
                );
                break;
            }

            case 'upgrade-available': {
                const { upgradeId, firmware, packages } = status.upgradeInfo;
                statusMessage = [
                    firmware ? (
                        <InlineNotification
                            key="info-firmware"
                            stretch
                            kind="info"
                            theme="inverse"
                            hideCloseButton
                            title={formatMessage({ defaultMessage: 'New firmware version available' })}
                        />
                    ) : null,
                    packages ? (
                        <InlineNotification
                            key="info-packages"
                            stretch
                            kind="info"
                            theme="inverse"
                            hideCloseButton
                            title={formatMessage(
                                {
                                    defaultMessage:
                                        '{count, plural, one {{count} package update available} other {{count} package updates available}}',
                                },
                                { count: packages.changes.length },
                            )}
                        >
                            <PackageChanges changes={packages.changes} />
                        </InlineNotification>
                    ) : null,
                    errors?.length ? this.#renderInlineNotificationError(errors, 'error-upgrade-available') : null,
                ];
                control = (
                    <div className={css.downloadConfirmBar}>
                        <Button
                            id={$('download-and-upgrade')}
                            kind="primary"
                            disabled={!upgradeId}
                            onClick={() => upgradeId && onStartUpgrade(upgradeId)}
                            children={formatMessage({ defaultMessage: 'Download & Upgrade' })}
                            renderIcon={IconUpgrade}
                        />
                    </div>
                );
                break;
            }

            case 'upgrading': {
                statusMessage = <UpgradeStepper {...status.progress} />;

                // Once started, the upgrade is committed — show it in a blocking overlay.
                overlayContent = (
                    <div className={css.overlayContent}>
                        <h1
                            className={css.title}
                            children={formatMessage({ defaultMessage: 'Upgrading the system' })}
                        />

                        {this.#renderFacts(versions, statusMessage)}

                        {changelog}
                    </div>
                );

                break;
            }

            case 'restarting': {
                let title: string;
                let body: ReactNode;
                switch (status.disruption) {
                    // Only firmware reboots reach this overlay, so an unset (UNSPECIFIED)
                    // disruption defaults to the full-reboot warning rather than
                    // under-warning as an app restart.
                    case pb.UpgradeDisruption.REBOOT:
                    case pb.UpgradeDisruption.UNSPECIFIED:
                        title = formatMessage({ defaultMessage: 'Braiins Deck is restarting…' });
                        body = <FormattedMessage defaultMessage="Please wait for the device to restart." />;
                        break;
                    case pb.UpgradeDisruption.APP_RESTART:
                        title = formatMessage({ defaultMessage: 'Restarting…' });
                        body = <FormattedMessage defaultMessage="Please wait for the app to restart." />;
                        break;
                    default:
                        return assertUnreachable(status.disruption, 'upgrade disruption');
                }

                overlayContent = (
                    <div className={css.overlayRestart}>
                        <h1 className={css.title}>
                            <IconRestart className={css.icon} />
                            <span children={title} />
                        </h1>

                        <p>
                            {body}
                            <br />
                            <Tick
                                intervalMs={this.#milis.restartingTick}
                                render={() => (
                                    <FormattedMessage
                                        defaultMessage="Time elapsed: {time}"
                                        values={{ time: this.#renderInterval(getTimestamp() - status.startTime) }}
                                    />
                                )}
                            />
                        </p>
                    </div>
                );
                break;
            }

            default:
                assertUnreachable(status, 'Upgrade from feeds / status kind');
        }

        return (
            <div className={css.updateContainer}>
                {this.#renderFacts(versions, statusMessage, downloadSize)}

                <div className={css.control} children={control} />

                {changelog}

                <Overlay inPortal isOpen={!!overlayContent} children={overlayContent} />
            </div>
        );
    }

    render() {
        const {
            intl: { formatMessage },
            automaticUpgrades,
        } = this.props;

        return (
            <section className={css.root}>
                <FieldSet title={formatMessage({ defaultMessage: 'Upgrade the System' })}>
                    <Field
                        title={formatMessage({
                            defaultMessage: 'Automatic Updates',
                        })}
                        description={formatMessage({
                            defaultMessage: 'Automatically install firmware updates when available.',
                        })}
                        disabled={automaticUpgrades.disabled}
                    >
                        {automaticUpgrades.value == null ? (
                            <ToggleSkeleton aria-label={formatMessage({ defaultMessage: 'Automatic Updates' })} />
                        ) : (
                            <Toggle
                                id={$('data-collection')}
                                size="md"
                                toggled={automaticUpgrades.value}
                                onToggle={automaticUpgrades.onChange}
                                disabled={automaticUpgrades.disabled}
                                labelA={formatMessage({ defaultMessage: 'Off' })}
                                labelB={formatMessage({ defaultMessage: 'On' })}
                            />
                        )}
                    </Field>

                    {this.#renderUpgradeBox()}
                </FieldSet>
            </section>
        );
    }
}

export function SectionUpgrade(props: SectionUpgradeProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
