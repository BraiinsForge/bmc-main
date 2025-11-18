import { Component, type HTMLAttributes, Fragment, useCallback } from 'react';
import { type IntlShape, useIntl, FormattedMessage } from 'react-intl';

// Libs
import type { iField } from '@/lib/form';
import { selfSelect } from '@/lib/react';

// App
import { URLS } from '@/constants';
import { getID } from '../const';
import * as pb from '@/proto';

// Components
import * as Icons from '@/components/images/icons';
import { TextInput, Layer } from '@carbon/react';
import { ScenePreview } from '@/pages/workspace/Display/components';
import { Cloud as IconCloud, type CarbonIconType } from '@carbon/react/icons';
import { ModalCustom, Tabs, type TabsProps, Deck as IconDeck, Link, Button } from '@/components';

// styles
import cn from 'clsx';
import css from './FormSceneSelect.scss';

export type SceneWidgetKind = ProtoOneofCase<pb.WidgetKind['value']>;
export type SceneKind = SceneWidgetKind;

export interface FormSceneSelectProps {
    isOpen: boolean;
    onClose(): void;
    onSelection(kind: SceneKind): void;

    remoteWidgetUrl: iField<string> & { onSubmit(): void };
    remoteWidgetRecents: pb.RemoteWidget[];
}
interface Props extends FormSceneSelectProps {
    intl: IntlShape;
}

interface State {
    tab: 'local' | 'remote';
    isLoadingRecentRemoteWidgets?: boolean;
}
const getInitialState = (): State => ({
    tab: 'local',
});

const $ = getID('scene-select-kind').get;
class View extends Component<Props, State> {
    readonly state = getInitialState();
    componentDidUpdate(prevProps: Props) {
        if (!this.props.isOpen && prevProps.isOpen) this.setState(getInitialState());
    }

    #tabChange = (tab: State['tab']) => this.setState({ tab });
    #tabs: TabsProps<State['tab']>['tabs'] = [
        {
            key: 'local',
            label: <TabFlap icon={IconDeck} name={this.props.intl.formatMessage({ defaultMessage: 'Local' })} />,
        },
        {
            key: 'remote',
            label: <TabFlap icon={IconCloud} name={this.props.intl.formatMessage({ defaultMessage: 'Remote' })} />,
        },
    ];

    #tabRenderLocal() {
        const { intl, onSelection } = this.props;
        const { formatMessage } = intl;

        return (
            <section className={css.grid}>
                <Cell
                    kind="clock"
                    icon={<Icons.WidgetClocks size={56} />}
                    description={formatMessage({
                        defaultMessage: 'You can choose between types of clocks - Flip, Digital, Analog',
                    })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="tickerBtc"
                    icon={<Icons.WidgetTicker size={56} />}
                    description={formatMessage({ defaultMessage: 'BTC price adjusted in 5 minute intervals.' })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="blockHeight"
                    icon={<Icons.WidgetBlockHeight size={56} />}
                    description={formatMessage({ defaultMessage: 'Show the current block height and timestamp.' })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="blockchainData"
                    icon={<Icons.WidgetBlockchainData size={56} />}
                    description={formatMessage({
                        defaultMessage: 'Get all the relevant information about bitcoin mining on one display.',
                    })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="braiinsPool"
                    icon={<Icons.WidgetPool size={56} />}
                    description={formatMessage({
                        defaultMessage: 'Display live metrics from your Braiins Pool mining account.',
                    })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="remoteImage"
                    icon={<Icons.WidgetRemoteImage size={56} />}
                    description={formatMessage({ defaultMessage: 'Display your own image.' })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="halvingCountdown"
                    icon={<Icons.WidgetHalvingCountdown size={56} />}
                    description={formatMessage({
                        defaultMessage: 'Countdown to the next Bitcoin halving event.',
                    })}
                    onSelection={onSelection}
                />

                <Cell
                    kind="countdown"
                    icon={<Icons.WidgetCountdown size={56} />}
                    description={formatMessage({
                        defaultMessage: 'Set a countdown timer to a specific date and time.',
                    })}
                    onSelection={onSelection}
                />
            </section>
        );
    }

    #reuseRecentRemoteWidget = (x: pb.RemoteWidget) => {
        const { remoteWidgetUrl } = this.props;
        remoteWidgetUrl.onChange?.(x.widgetUrl);
        setTimeout(remoteWidgetUrl.onSubmit, 50);
    };
    #tabRenderRemote() {
        const { intl, remoteWidgetUrl, remoteWidgetRecents } = this.props;
        const { formatMessage } = intl;

        return (
            <div className={css.remoteSection}>
                <section className={css.remoteInstall}>
                    <div className={css.remoteIntro}>
                        <FormattedMessage
                            tagName="p"
                            defaultMessage="Remote widgets run on server and Braiins DECK is retrieving widget snapshots every so often as an image."
                        />
                        <br />
                        <FormattedMessage
                            tagName="p"
                            defaultMessage="View <a>Official Braiins DECK Widgets Directory</a>"
                            values={{
                                a: x => (
                                    <Link
                                        external
                                        href={URLS.external.widgetsDirectory}
                                        children={x}
                                        style={{ fontWeight: 'bold' }}
                                    />
                                ),
                            }}
                        />
                    </div>

                    <Layer level={1}>
                        <div className={css.remoteInputBox}>
                            <TextInput
                                id={$('remote-widget-url')}
                                labelText={formatMessage({ defaultMessage: 'Add New Widget with URL' })}
                                placeholder={new URL('...', URLS.external.widgetsDirectory).href}
                                value={remoteWidgetUrl.value || ''}
                                onChange={e => remoteWidgetUrl.onChange?.(e.target.value)}
                                invalid={!!remoteWidgetUrl.error}
                                invalidText={remoteWidgetUrl.error}
                                disabled={remoteWidgetUrl.disabled}
                                onFocus={selfSelect}
                            />
                            <Button
                                id={$('remote-widget-load-button')}
                                kind="primary"
                                size="md"
                                children={formatMessage({ defaultMessage: 'Load Widget' })}
                                disabled={remoteWidgetUrl.disabled || !!remoteWidgetUrl.error}
                                onClick={remoteWidgetUrl.onSubmit}
                            />
                        </div>
                    </Layer>
                </section>

                <section className={css.remoteRecent}>
                    <h1
                        children={formatMessage(
                            { defaultMessage: 'Recently Used Remote Widgets ({count})' },
                            { count: remoteWidgetRecents.length },
                        )}
                    />
                    <div className={css.grid}>
                        {remoteWidgetRecents.length ? (
                            remoteWidgetRecents.map((x, i) => {
                                return (
                                    <Cell
                                        key={i}
                                        kind={x}
                                        icon={<ScenePreview kind={{ case: 'remoteWidget', value: x }} />}
                                        description={x.description}
                                        onSelection={this.#reuseRecentRemoteWidget}
                                    />
                                );
                            })
                        ) : (
                            <CellSkeletonSet count={3} />
                        )}
                    </div>
                </section>
            </div>
        );
    }

    render() {
        const { isOpen, onClose, intl } = this.props;
        const { tab } = this.state;
        const { formatMessage } = intl;

        let content: ReactNode;
        switch (tab) {
            case 'local':
                content = this.#tabRenderLocal();
                break;

            case 'remote':
                content = this.#tabRenderRemote();
                break;

            // no default
        }

        return (
            <ModalCustom
                id={$('modal')}
                open={isOpen}
                size="lg"
                title={formatMessage({ defaultMessage: 'Add New Widget' })}
                selectorPrimaryFocus="input"
                onClose={onClose}
                cancelBodyOverflowShadow
                bodyClassName={css.dialogBody}
            >
                {/* Mitigation for unwanted and otherwise seamingly unpreventable focus first button. */}
                <input type="hidden" />

                <Tabs<State['tab']> tabs={this.#tabs} activeTab={tab} onChange={this.#tabChange} />

                {content}
            </ModalCustom>
        );
    }
}

interface TabFlapProps {
    name: string;
    icon: CarbonIconType;
}
function TabFlap(props: TabFlapProps) {
    const { name, icon: Icon } = props;
    return (
        <div className={css.tab}>
            <Icon />
            <span children={name} />
        </div>
    );
}

interface CellProps<T extends SceneKind | pb.RemoteWidget> {
    kind: T;
    icon: ReactElement;
    description: null | string;
    onSelection(kind: T): void;
}
function Cell<T extends SceneKind | pb.RemoteWidget>(props: CellProps<T>) {
    const { kind, icon, description, onSelection } = props;
    const intl = useIntl();

    const title: string = typeof kind === 'string' ? String(pb.sceneTitle(intl, kind)) : kind.name;
    const select = useCallback(() => onSelection(kind), [kind, onSelection]);

    return (
        <button type="button" onClick={select} className={css.cell}>
            <aside className={css.icon} children={icon} />
            <main>
                <div className={css.title} children={title} />
                <div className={css.desc} children={description} />
            </main>
        </button>
    );
}

function CellSkeleton(props: HTMLAttributes<HTMLDivElement>) {
    const { className, ...rest } = props;
    return <div {...rest} tabIndex={-1} className={cn(css.cell, css.skeleton, className)} />;
}
function CellSkeletonSet(props: { count: number }) {
    const { count } = props;
    const opacityBase = 0.7;
    const opacityStep = 1 / (count + 1);

    return (
        <Fragment
            children={Array.from({ length: count }).map((_, i) => (
                <CellSkeleton key={i} style={{ opacity: opacityBase - i * opacityStep }} />
            ))}
        />
    );
}

export function FormSceneSelect(props: FormSceneSelectProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
