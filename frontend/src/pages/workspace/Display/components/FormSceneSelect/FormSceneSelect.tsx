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

import { Component, type HTMLAttributes, Fragment, useCallback } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

// App
import { getID } from '../const';
import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

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

// Section order in the picker; widgets fall under `MISC` (rendered last) when
// their category is unset or not one yet surfaced here.
const CATEGORY_ORDER: pb.WidgetCategory[] = [
    pb.WidgetCategory.MINING,
    pb.WidgetCategory.FINANCE,
    pb.WidgetCategory.CLOCK,
    pb.WidgetCategory.WEATHER,
    pb.WidgetCategory.CALENDAR,
    pb.WidgetCategory.SPACE,
    pb.WidgetCategory.MEDIA,
    pb.WidgetCategory.KNOWLEDGE,
    pb.WidgetCategory.SPORTS,
    pb.WidgetCategory.UTILITY,
    pb.WidgetCategory.MISC,
];

function bucketOf(category: pb.WidgetCategory): pb.WidgetCategory {
    return CATEGORY_ORDER.includes(category) ? category : pb.WidgetCategory.MISC;
}

function categoryLabel(intl: IntlShape, category: pb.WidgetCategory): string {
    const { formatMessage } = intl;
    switch (category) {
        case pb.WidgetCategory.MINING:
            return formatMessage({ defaultMessage: 'Mining' });

        case pb.WidgetCategory.FINANCE:
            return formatMessage({ defaultMessage: 'Finance' });

        case pb.WidgetCategory.CLOCK:
            return formatMessage({ defaultMessage: 'Clock' });

        case pb.WidgetCategory.WEATHER:
            return formatMessage({ defaultMessage: 'Weather' });

        case pb.WidgetCategory.CALENDAR:
            return formatMessage({ defaultMessage: 'Calendar' });

        case pb.WidgetCategory.SPACE:
            return formatMessage({ defaultMessage: 'Space' });

        case pb.WidgetCategory.MEDIA:
            return formatMessage({ defaultMessage: 'Media' });

        case pb.WidgetCategory.KNOWLEDGE:
            return formatMessage({ defaultMessage: 'Knowledge' });

        case pb.WidgetCategory.SPORTS:
            return formatMessage({ defaultMessage: 'Sports' });

        case pb.WidgetCategory.UTILITY:
            return formatMessage({ defaultMessage: 'Utility' });

        case pb.WidgetCategory.MISC:
        case pb.WidgetCategory.UNSPECIFIED:
            return formatMessage({ defaultMessage: 'Other' });

        default:
            return assertUnreachable(category, 'widget category');
    }
}

interface CategorySection {
    category: pb.WidgetCategory;
    widgets: pb.WidgetManifest[];
}

// Non-empty category sections in display order, widgets name-sorted within each.
export function groupByCategory(widgets: pb.WidgetManifest[]): CategorySection[] {
    return CATEGORY_ORDER.map(category => ({
        category,
        widgets: widgets.filter(w => bucketOf(w.category) === category).sort((a, b) => a.name.localeCompare(b.name)),
    })).filter(section => section.widgets.length > 0);
}

// Sections kept by the filter pills: an empty selection means "show everything".
export function visibleSections(sections: CategorySection[], selected: Set<pb.WidgetCategory>): CategorySection[] {
    return selected.size === 0 ? sections : sections.filter(s => selected.has(s.category));
}

interface State {
    // Categories the user has toggled on; empty means no filter (show all).
    selected: Set<pb.WidgetCategory>;
}

const $ = getID('scene-select-kind').get;
class View extends Component<Props, State> {
    state: State = { selected: new Set() };

    #handleManifestSelect = (manifest: pb.WidgetManifest) => {
        this.props.onManifestSelection(manifest);
    };

    #toggleCategory = (category: pb.WidgetCategory) => {
        this.setState(prev => {
            const selected = new Set(prev.selected);
            if (selected.has(category)) {
                selected.delete(category);
            } else {
                selected.add(category);
            }
            return { selected };
        });
    };

    // Reset the filter when the dialog closes so it reopens unfiltered.
    componentDidUpdate(prev: Props) {
        if (prev.isOpen && !this.props.isOpen && this.state.selected.size > 0) {
            this.setState({ selected: new Set() });
        }
    }

    render() {
        const { isOpen, onClose, intl, manifestWidgets, isLoading } = this.props;
        const { formatMessage } = intl;
        const { selected } = this.state;

        const sections = groupByCategory(manifestWidgets);
        // Drop selected categories whose section is gone (e.g. the last widget in
        // it hot-reloaded away): their pill no longer renders, so keeping them
        // selected would strand the user on an empty, un-clearable view.
        const available = new Set(sections.map(s => s.category));
        const activeSelected = new Set([...selected].filter(c => available.has(c)));

        const body =
            manifestWidgets.length > 0 ? (
                <Fragment>
                    {sections.length > 1 ? (
                        <CategoryFilter
                            sections={sections}
                            selected={activeSelected}
                            intl={intl}
                            onToggle={this.#toggleCategory}
                        />
                    ) : null}
                    {visibleSections(sections, activeSelected).map(({ category, widgets }) => (
                        <section key={category} className={css.manifestSection}>
                            <h1 children={categoryLabel(intl, category)} />
                            <div className={css.grid}>
                                {widgets.map(m => (
                                    <Cell key={m.uid} manifest={m} onSelection={this.#handleManifestSelect} />
                                ))}
                            </div>
                        </section>
                    ))}
                </Fragment>
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

interface CategoryFilterProps {
    sections: CategorySection[];
    selected: Set<pb.WidgetCategory>;
    intl: IntlShape;
    onToggle(category: pb.WidgetCategory): void;
}
// Toggleable category pills with per-category counts; a pill is active when its
// category is in `selected`. Multi-select, mirroring the public widget gallery.
function CategoryFilter(props: CategoryFilterProps) {
    const { sections, selected, intl, onToggle } = props;
    return (
        <div className={css.filters}>
            {sections.map(({ category, widgets }) => (
                <button
                    key={category}
                    type="button"
                    aria-pressed={selected.has(category)}
                    className={cn(css.pill, selected.has(category) && css.pillActive)}
                    onClick={() => onToggle(category)}
                    children={`${categoryLabel(intl, category)} (${widgets.length})`}
                />
            ))}
        </div>
    );
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
