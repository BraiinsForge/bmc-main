import { Component, type HTMLAttributes, Fragment, useCallback } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

// App
import { getID } from '../const';
import type * as pb from '@/proto';

// Components
import { Apps as IconApps } from '@carbon/react/icons';
import { Image, ModalCustom } from '@/components';
import { WidgetName } from '../WidgetName';

// styles
import cn from 'clsx';
import css from './FormSceneSelect.scss';

export interface FormSceneSelectProps {
    isOpen: boolean;
    onClose(): void;
    onManifestSelection(manifest: pb.WidgetManifest): void;

    manifestWidgets: pb.WidgetManifest[];
    isLoading?: boolean;
}
interface Props extends FormSceneSelectProps {
    intl: IntlShape;
}

const $ = getID('scene-select-kind').get;
class View extends Component<Props> {
    #handleManifestSelect = (manifest: pb.WidgetManifest) => {
        this.props.onManifestSelection(manifest);
    };

    render() {
        const { isOpen, onClose, intl, manifestWidgets, isLoading } = this.props;
        const { formatMessage } = intl;

        const body =
            manifestWidgets.length > 0 ? (
                <section className={css.grid}>
                    {manifestWidgets.map(m => (
                        <Cell key={m.uid} manifest={m} onSelection={this.#handleManifestSelect} />
                    ))}
                </section>
            ) : isLoading ? (
                <CellSkeletonSet count={3} />
            ) : (
                <EmptyState
                    text={formatMessage({
                        defaultMessage: 'No widgets are installed.',
                    })}
                />
            );

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
                {body}
            </ModalCustom>
        );
    }
}

function EmptyState(props: { text: string }) {
    return <div className={css.empty} children={props.text} />;
}

interface CellProps {
    manifest: pb.WidgetManifest;
    onSelection(manifest: pb.WidgetManifest): void;
}
function Cell(props: CellProps) {
    const { manifest, onSelection } = props;
    const select = useCallback(() => onSelection(manifest), [manifest, onSelection]);

    return (
        <button type="button" onClick={select} className={css.cell}>
            <aside
                className={css.icon}
                children={
                    <Image
                        src={manifest.iconUrl || null}
                        alt={manifest.name}
                        width={56}
                        height={56}
                        render={(img, failed) => (failed ? <IconApps size={56} /> : img())}
                    />
                }
            />
            <main>
                <div className={css.title}>
                    <WidgetName name={manifest.name} subname={manifest.subname} />
                </div>
                <div className={css.desc} children={manifest.description} />
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
