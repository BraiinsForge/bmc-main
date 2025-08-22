import { Component } from 'react';
import Markdown from 'markdown-it';
import { formatDuration } from 'date-fns';
import { FormattedMessage, useIntl, type IntlShape } from 'react-intl';

// App
import * as pb from '@/proto';

// Lib
import { type iField, getID } from '@/lib/form';

// Components
import {
    Field,
    FieldSet,
    Button,
    InlineNotification,
    InlineNotificationsGroup,
    Html,
    Progressbar,
    Overlay,
    Percentage,
    Tick,
} from '@/components';
import { InlineLoading, Toggle } from '@carbon/react';
import {
    Upgrade as IconUpgrade,
    ChevronDown,
    ChevronUp,
    Error as IconError,
    Checkmark as IconCheckmark,
    Restart as IconRestart,
} from '@carbon/react/icons';

// Styles
import cn from 'clsx';
import C from '@/styles/colors';
import css from './SectionUpgrade.scss';
import { assertUnreachable } from '@/lib/ts';
import { getTimestamp } from '@/lib/time';

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
          kind: 'downloading';
          upgradeInfo: pb.CheckForUpgradeResponse;
          downloadProgress: pb.DownloadProgress;
      }
    | {
          kind: 'installing';
          upgradeInfo: pb.CheckForUpgradeResponse;
          startTime: Timestamp;
      }
    | {
          kind: 'restarting';
          upgradeInfo: null;
          startTime: Timestamp;
      };

export interface SectionUpgradeProps {
    automaticUpgrades: iField<boolean>;
    versionCurrent: null | string;

    status: null | UpgradeFromFeedStatus;
    errors?: Maybe<string[]>;

    onCheckUpdates(): void;
    onDownload(hash: string): void;
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

const $ = getID('settings', 'updates').get;
class View extends Component<Props, State> {
    #milis = {
        installingTick: 150,
        restartingTick: 1e3,
    };
    #NA = <span className={css.placeholder} children="N/A" />;

    readonly state = getInitialState();

    componentDidUpdate(prevProps: Props) {
        const { status } = this.props;
        if (
            status?.kind !== prevProps.status?.kind ||
            status?.upgradeInfo?.latestRelease?.hash !== prevProps.status?.upgradeInfo?.latestRelease?.hash
        ) {
            this.setState({ isChangelogExpanded: false });
        }
    }

    #renderProgressBar(
        percentage: number,
        label: ReactNode,
        color: string,
        extra?: {
            footer?: ReactNode;
            error?: ReactNode;
            errorTitle?: ReactNode;
        },
    ): ReactElement {
        const value = {
            value: percentage,
            color: color,
            animate: true,
        };
        let title: ReactNode = label;
        let icon: ReactNode;
        let footerContent: ReactNode;
        const classNames: string[] = [css.progressbar];

        if (extra?.error) {
            title = extra.errorTitle ?? title;
            icon = <IconError fill={C.alertRed} />;
            footerContent = extra.error;
            value.color = C.alertRed;
            value.animate = false;
            classNames.push(css.invalid);
        } else {
            footerContent = extra?.footer;
        }
        if (percentage === 100) icon ??= <IconCheckmark fill={C.alertGreen} />;

        return (
            <div className={cn(classNames)}>
                {icon ? <div className={css.icon} children={icon} /> : null}
                <Progressbar label={title} labelPosition="top-left" valueUpperBound={100} values={[value]} />
                {footerContent != null ? <div className={css.footer} children={footerContent} /> : null}
            </div>
        );
    }

    #txt = {
        estDuration: (
            <div
                role="presentation"
                className={css.estimatedDuration}
                children={this.props.intl.formatMessage({ defaultMessage: 'Est. upgrade time: 1 minute' })}
            />
        ),
    };
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

        const upgradeInfo = status?.upgradeInfo;
        if (!upgradeInfo) return null;
        const { previousReleases, latestRelease } = upgradeInfo;
        if (!latestRelease) return null;

        let expandedConent: ReactNode;
        let expanderButton: ReactNode;
        if (previousReleases.length) {
            if (isChangelogExpanded) {
                expandedConent = previousReleases.map(x => this.#renderDescription(x.version, x.description));
            }

            expanderButton = (
                <Button
                    id={$('changelog-expander')}
                    kind="tertiary"
                    onClick={this.#toggleChangelog}
                    children={
                        isChangelogExpanded
                            ? intl.formatMessage({ defaultMessage: 'Hide Older Updates' })
                            : intl.formatMessage(
                                  { defaultMessage: '{count, plural, one {+# Older Update} other {+# Older Updates}}' },
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

                {this.#renderDescription(latestRelease.version, latestRelease.description)}

                {expandedConent}
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
    #renderFacts = (versionCurrent: Maybe<ReactNode>, versionLatest: Maybe<ReactNode>, statusMessage: ReactNode) => {
        const { formatMessage } = this.props.intl;

        return (
            <div className={css.facts}>
                <table>
                    <tbody>
                        <tr>
                            <th scope="row" children={formatMessage({ defaultMessage: 'Your version:' })} />
                            <td children={versionCurrent ?? 'N/A'} />
                        </tr>
                        <tr>
                            <th scope="row" children={formatMessage({ defaultMessage: 'Latest available version:' })} />
                            <td children={versionLatest ?? 'N/A'} />
                        </tr>
                    </tbody>
                </table>

                <div className={css.statusMessage} children={statusMessage} />
            </div>
        );
    };
    #renderInterval = (seconds: number): ReactNode => {
        return formatDuration({ seconds: Math.abs(seconds) }, { zero: true, format: ['minutes', 'seconds'] });
    };
    #renderSizeMB = (size: number): string => {
        return `${Number(size).toFixed(2)} MB`;
    };

    #renderUpgradeBox(): ReactNode {
        const {
            intl: { formatMessage },

            status,
            errors,
            versionCurrent,

            onCheckUpdates,
            onDownload,
        } = this.props;

        const versionLatest: ReactNode = status?.upgradeInfo?.latestRelease?.version ?? versionCurrent ?? this.#NA;

        // Shis one is defined as static because it is used in both overlay and base
        // layers and the only status dependent part is the loading spinner
        const latestVersionCellContent =
            status?.kind === 'checking-upgrade' ? (
                <InlineLoading
                    status="active"
                    description={formatMessage({ defaultMessage: 'Checking for updates…' })}
                />
            ) : (
                (versionLatest ?? 'N/A')
            );
        const changelog = this.#renderChangelog();

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
                        children={formatMessage({ defaultMessage: "You're running the latest firmware" })}
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
                const hash = status.upgradeInfo.latestRelease?.hash as string;
                statusMessage = [
                    <InlineNotification
                        key="info"
                        stretch
                        kind="info"
                        theme="inverse"
                        hideCloseButton
                        title={formatMessage({ defaultMessage: 'New version available' })}
                    />,
                    errors?.length ? this.#renderInlineNotificationError(errors, 'error-upgrade-available') : null,
                ];
                control = (
                    <div className={css.downloadConfirmBar}>
                        <Button
                            id={$('download-and-upgrade')}
                            kind="primary"
                            onClick={() => onDownload(hash)}
                            children={formatMessage({ defaultMessage: 'Download & Upgrade firmware' })}
                            renderIcon={IconUpgrade}
                        />
                        {this.#txt.estDuration}
                    </div>
                );
                break;
            }

            case 'downloading': {
                const { totalMb, downloadedMb } = status.downloadProgress;
                const percentage = (downloadedMb / totalMb) * 100;

                statusMessage = this.#renderProgressBar(
                    percentage,
                    formatMessage(
                        { defaultMessage: 'Downloading {progress}…' },
                        {
                            progress: <Percentage value={percentage} upperValueBound={100} round={0} />,
                        },
                    ),
                    C.accentViolet,
                    {
                        footer: formatMessage(
                            { defaultMessage: '{done} of {total}' },
                            {
                                done: this.#renderSizeMB(downloadedMb),
                                total: this.#renderSizeMB(totalMb),
                            },
                        ),
                        error: pb.renderFieldErrorsAsList(errors),
                        errorTitle: formatMessage({ defaultMessage: 'Download failed' }),
                    },
                );
                break;
            }

            case 'installing': {
                statusMessage = (
                    <Tick
                        intervalMs={this.#milis.installingTick}
                        render={() => {
                            const now = Math.floor(Date.now() / 1e3);
                            const secondsTotal: number = 60;

                            const secondsPassed: number = now - status.startTime;
                            const secondsRemaining: number = Math.max(0, secondsTotal - secondsPassed);

                            const percentage = Math.min((secondsPassed / secondsTotal) * 100, 100);
                            const errorTitle: string = formatMessage({ defaultMessage: 'Installation failed' });

                            if (percentage === 100) {
                                return this.#renderProgressBar(
                                    100,
                                    formatMessage({ defaultMessage: 'Installation Finished' }),
                                    C.alertGreen,
                                    {
                                        footer: formatMessage({ defaultMessage: 'Upgrade Successfull' }),
                                        error: pb.renderFieldErrorsAsList(errors),
                                        errorTitle,
                                    },
                                );
                            }

                            return this.#renderProgressBar(
                                percentage,
                                formatMessage({ defaultMessage: 'Installing…' }),
                                C.alertGreen,
                                {
                                    footer: formatMessage(
                                        { defaultMessage: '{time} remaining' },
                                        { time: this.#renderInterval(secondsRemaining) },
                                    ),
                                    error: pb.renderFieldErrorsAsList(errors),
                                    errorTitle,
                                },
                            );
                        }}
                    />
                );

                // All of constructed content is shown in the base,
                // but also in an overlay that blocks the rest of the UI
                overlayContent = (
                    <div className={css.overlayContent}>
                        <h1 className={css.title} children={formatMessage({ defaultMessage: 'Installing Upgrade' })} />

                        {this.#renderFacts(versionCurrent, latestVersionCellContent, statusMessage)}

                        <div className={css.control} children={control} />

                        {changelog}
                    </div>
                );

                break;
            }

            case 'restarting': {
                overlayContent = (
                    <div className={css.overlayRestart}>
                        <h1 className={css.title}>
                            <IconRestart className={css.icon} />
                            <span children={formatMessage({ defaultMessage: 'Braiins Deck installion…' })} />
                        </h1>

                        <p>
                            <FormattedMessage defaultMessage="Please wait for the device restart to complete." />
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
                {this.#renderFacts(versionCurrent, latestVersionCellContent, statusMessage)}

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
                        <Toggle
                            id={$('data-collection')}
                            size="md"
                            toggled={!!automaticUpgrades.value}
                            onToggle={automaticUpgrades.onChange}
                            disabled={automaticUpgrades.disabled}
                            labelA={formatMessage({ defaultMessage: 'Off' })}
                            labelB={formatMessage({ defaultMessage: 'On' })}
                        />
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
