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

import { Fragment, useState } from 'react';
import { useIntl } from 'react-intl';
import { Tag } from '@carbon/react';
import { ChevronDown, ChevronUp } from '@carbon/react/icons';

import type * as pb from '@/proto';
import { Markdown } from '@/components';
import css from './PackageChanges.scss';
import cn from 'clsx';

// Sort (not group) by category — widget / core / dev / …
// — so like kinds cluster; uncategorized entries sort last, ties broken by name.
// The category itself surfaces as an inline tag on each row.
function sortPackageChanges(changes: pb.PackageChange[]): pb.PackageChange[] {
    return changes.toSorted(
        (a, b) => (a.category ?? '￿').localeCompare(b.category ?? '￿') || a.name.localeCompare(b.name),
    );
}

export interface PackageChangesProps {
    changes: pb.PackageChange[];
}

// The set of app-package changes bundled in an upgrade:
//  name, category tag, and version transition per row,
//  with an expandable row revealing that package's
//  changelog (when it has one).
export function PackageChanges({ changes }: PackageChangesProps) {
    const { formatMessage } = useIntl();
    const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

    const toggle = (name: string) =>
        setExpanded(prev => {
            const next = new Set(prev);
            if (next.has(name)) next.delete(name);
            else next.add(name);
            return next;
        });

    // Distinguish the two "empty side" operations from each other and from a
    // muted "no data" — a missing `from` is a fresh install, a missing `to` a removal.
    const newLabel = <span className={css.operation} children={formatMessage({ defaultMessage: 'new' })} />;
    const removedLabel = <span className={css.operation} children={formatMessage({ defaultMessage: 'removed' })} />;

    return (
        <table className={css.table}>
            <tbody
                children={sortPackageChanges(changes).map((change, i) => {
                    const changelog = change.changelog;
                    const isOpen = expanded.has(change.name);

                    return (
                        <Fragment key={i}>
                            <tr
                                className={cn(css.changeRow, changelog && css.expandable)}
                                onClick={changelog ? () => toggle(change.name) : undefined}
                            >
                                <td className={css.expandCell}>
                                    {changelog ? (
                                        <button
                                            type="button"
                                            className={css.expandButton}
                                            aria-expanded={isOpen}
                                            aria-label={formatMessage(
                                                { defaultMessage: 'Toggle changelog for {name}' },
                                                { name: change.name },
                                            )}
                                            onClick={event => {
                                                // Row handles the click too; keep the button's
                                                // keyboard control from double-firing via bubbling.
                                                event.stopPropagation();
                                                toggle(change.name);
                                            }}
                                            children={isOpen ? <ChevronUp /> : <ChevronDown />}
                                        />
                                    ) : null}
                                </td>
                                <th scope="row">
                                    <span children={change.name} />
                                    {change.category ? (
                                        <Tag
                                            size="sm"
                                            type="cool-gray"
                                            className={css.tag}
                                            children={change.category}
                                        />
                                    ) : null}
                                </th>
                                <td className={css.from} children={change.versionFrom ?? newLabel} />
                                <td className={css.arrow} children="→" />
                                <td className={css.into} children={change.versionTo ?? removedLabel} />
                            </tr>

                            {changelog && isOpen ? (
                                <tr>
                                    <td className={css.expandCell} />
                                    <td className={css.changelogCell} colSpan={4}>
                                        <Markdown source={changelog} />
                                    </td>
                                </tr>
                            ) : null}
                        </Fragment>
                    );
                })}
            />
        </table>
    );
}
