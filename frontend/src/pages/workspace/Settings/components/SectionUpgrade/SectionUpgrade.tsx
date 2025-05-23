import { Component } from 'react';
import Markdown from 'markdown-it';
import { useIntl, type IntlShape } from 'react-intl';

import type * as pb from '@/proto';
import { Form, type iField, getID } from '@/lib/form';

import { Field } from '../Field';
import { FieldSet } from '../FieldSet';

import { Button, InlineNotification, Html } from '@/components';
import { Toggle } from '@carbon/react';
import { Upgrade as IconUpgrade, ChevronDown, ChevronUp } from '@carbon/react/icons';

// Styles
import css from './SectionUpgrade.scss';

export interface SectionUpgradeProps {
    automaticUpgrades: iField<boolean>;

    versionCurrent: null | string;
    upgradeInfo: null | pb.CheckForUpgradeResponse;
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

const $id = getID('settings', 'updates');
const NA = <span className={css.placeholder} children="N/A" />;

class View extends Component<Props, State> {
    readonly state = getInitialState();

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
        const { upgradeInfo, intl } = this.props;
        const { isChangelogExpanded } = this.state;

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

    render() {
        const {
            intl: { formatMessage },
            upgradeInfo,
            automaticUpgrades,
            versionCurrent,
        } = this.props;

        return (
            <section className={css.root}>
                <FieldSet title={formatMessage({ defaultMessage: 'Update' })}>
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
                            id={$id.get('data-collection')}
                            size="md"
                            toggled={!!automaticUpgrades.value}
                            onToggle={automaticUpgrades.onChange}
                            disabled={automaticUpgrades.disabled}
                            labelA={formatMessage({ defaultMessage: 'Off' })}
                            labelB={formatMessage({ defaultMessage: 'On' })}
                        />
                    </Field>

                    {upgradeInfo == null ? null : (
                        <div className={css.updateContainer}>
                            <Form className={css.updateForm}>
                                <table>
                                    <tbody>
                                        <tr>
                                            <th
                                                scope="row"
                                                children={formatMessage({ defaultMessage: 'Your version:' })}
                                            />
                                            <td children={versionCurrent ?? NA} />
                                        </tr>
                                        <tr>
                                            <th
                                                scope="row"
                                                children={formatMessage({
                                                    defaultMessage: 'Latest version available:',
                                                })}
                                            />
                                            <td
                                                children={upgradeInfo?.latestRelease?.version ?? versionCurrent ?? NA}
                                            />
                                        </tr>
                                    </tbody>
                                </table>

                                {upgradeInfo?.latestRelease ? (
                                    <InlineNotification
                                        title={formatMessage({ defaultMessage: 'New Version Available' })}
                                        kind="info"
                                        theme="auto"
                                        stretch
                                        hideCloseButton
                                        style={{ marginTop: '0.5rem' }}
                                    />
                                ) : null}
                                <footer className={css.updateFormFooter}>
                                    <Button
                                        kind="primary"
                                        icon={IconUpgrade}
                                        children={formatMessage({ defaultMessage: 'Download & Upgrade BMC' })}
                                    />
                                    <div
                                        role="presentation"
                                        className={css.estimatedDuration}
                                        children={formatMessage({ defaultMessage: 'Est. upgrade time: 1 minute' })}
                                    />
                                </footer>
                            </Form>

                            {this.#renderChangelog()}
                        </div>
                    )}
                </FieldSet>
            </section>
        );
    }
}

export function SectionUpgrade(props: SectionUpgradeProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
