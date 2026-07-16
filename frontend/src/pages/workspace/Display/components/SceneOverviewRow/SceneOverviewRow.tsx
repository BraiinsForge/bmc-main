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

import { Component, Fragment, type DetailedHTMLProps, type HTMLAttributes } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

// Lib
import { getID } from '../const';
import { selfSelect } from '@/lib/react';

// Components
import { Button, CarbonFormField } from '@/components';
import { SceneTypeIcons, type SceneTypeIconsProps } from '../SceneTypeIcons';
import { Toggle, NumberInput } from '@carbon/react';
import {
    Draggable as IconDraggable,
    TrashCan as IconDelete,
    Copy as IconClone,
    Restart as IconRestart,
} from '@carbon/react/icons';

// Styles
import cn, { type ClassValue } from 'clsx';
import css from './SceneOverviewRow.scss';
import { assertUnreachable } from '@/lib/ts.ts';

export interface SceneOverviewRowProps {
    id: string;
    // List is settling a clone placeholder; disable this row's controls.
    locked?: boolean;
    enabled: boolean;
    onToggle(id: string, value: boolean): void;

    cycleEnabled: boolean;
    cycleDurationValue: Maybe<string | number>;
    cycleDurationDefault: string | number;
    onDurationChange(id: string, duration: string): void;

    onEdit(id: string): void;
    onClone(id: string): void;
    onDelete(id: string): void;
    onReload?(id: string): void;

    icon: ReactNode;
    title: ReactNode;
    description: ReactNode;
    type: Pick<SceneTypeIconsProps, 'night'>;

    // DnD
    dndRootProps?: DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>;
    dndDragHandleProps?: DetailedHTMLProps<HTMLAttributes<HTMLDivElement>, HTMLDivElement>;

    // Visual, DOM
    layout: 'card' | 'row';
    className?: string;
    style?: CSSProperties;
    children?: ReactNode;
}
interface Props extends SceneOverviewRowProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    #id = (...suffix: Array<string | number>) => {
        return getID('scene', 'overview', 'row', this.props.id).get(...suffix);
    };

    #toggle = (value: boolean): void => {
        const { onToggle, id } = this.props;
        onToggle(id, value);
    };
    #durationChange = (_: any, info: { value: string | number }): void => {
        const { onDurationChange, cycleEnabled, id } = this.props;
        if (cycleEnabled) onDurationChange(id, String(info.value));
    };
    #edit = (): void => {
        const { onEdit, id } = this.props;
        onEdit(id);
    };
    #clone = (): void => {
        const { onClone, id } = this.props;
        onClone(id);
    };
    #delete = (): void => {
        const { onDelete, id } = this.props;
        onDelete(id);
    };
    #reload = (): void => {
        const { onReload, id } = this.props;
        if (onReload) onReload(id);
    };

    Handle = (): ReactElement => {
        const { dndDragHandleProps } = this.props;

        return (
            <div
                {...dndDragHandleProps}
                className={cn(css.dragHandle, dndDragHandleProps?.className)}
                children={<IconDraggable />}
            />
        );
    };
    DurationInput = (props: { label: 'top' | 'none' }): ReactElement => {
        const { enabled, cycleEnabled, cycleDurationValue, cycleDurationDefault, locked } = this.props;
        const { formatMessage } = this.props.intl;

        return (
            <div className={cn(css.duration, !cycleEnabled && css.disabled)}>
                <NumberInput
                    disabled={!enabled || !cycleEnabled || locked}
                    id={this.#id('duration')}
                    min={1}
                    step={5}
                    allowEmpty
                    disableWheel
                    stepStartValue={Number.parseInt(String(cycleDurationDefault || 0), 10)}
                    placeholder={String(cycleDurationDefault)}
                    value={cycleDurationValue ?? ''}
                    onChange={this.#durationChange}
                    onFocus={selfSelect}
                    label={props.label === 'top' ? formatMessage({ defaultMessage: 'Duration (s)' }) : undefined}
                />
            </div>
        );
    };
    Icon = (): ReactElement => {
        const { icon } = this.props;
        return <div className={css.icon} children={icon} />;
    };
    Toggler = (props: { labeled?: boolean }): ReactElement => {
        const { enabled, intl, locked } = this.props;
        const { formatMessage } = intl;
        const { labeled } = props;

        return (
            <div className={css.toggle}>
                <Toggle
                    id={this.#id('enabled')}
                    size="md"
                    hideLabel={!labeled}
                    disabled={locked}
                    labelA={formatMessage({ defaultMessage: 'Off' })}
                    labelB={formatMessage({ defaultMessage: 'On' })}
                    toggled={enabled}
                    onToggle={this.#toggle}
                />
            </div>
        );
    };
    Labels = (props: { types: 'below' | 'inline' }): ReactElement => {
        const { title, description, type } = this.props;
        const { types } = props;

        const $types = <SceneTypeIcons {...type} className={css.types} />;

        return (
            <div className={css.labels}>
                <div className={css.title}>
                    <span children={title} />
                    {types === 'inline' && $types}
                </div>
                <div className={css.details} children={description} />
                {types === 'below' && $types}
            </div>
        );
    };
    Actions = (): ReactElement => {
        const { intl, onReload, locked } = this.props;
        const { formatMessage } = intl;

        return (
            <div className={css.actions}>
                {typeof onReload === 'function' && (
                    <Button
                        id={this.#id('reload')}
                        size="sm"
                        kind="ghost"
                        hasIconOnly
                        disabled={locked}
                        icon={IconRestart}
                        tooltipPosition="bottom"
                        title={formatMessage({ defaultMessage: 'Reload' })}
                        onClick={this.#reload}
                    />
                )}
                <Button
                    id={this.#id('clone')}
                    size="sm"
                    kind="ghost"
                    hasIconOnly
                    disabled={locked}
                    icon={IconClone}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Clone' })}
                    onClick={this.#clone}
                />
                <Button
                    id={this.#id('delete')}
                    size="sm"
                    kind="ghost"
                    hasIconOnly
                    disabled={locked}
                    icon={IconDelete}
                    tooltipPosition="bottom"
                    title={formatMessage({ defaultMessage: 'Delete' })}
                    onClick={this.#delete}
                />
                <Button
                    id={this.#id('edit')}
                    size="sm"
                    kind="primary"
                    disabled={locked}
                    tooltipPosition="bottom"
                    children={formatMessage({ defaultMessage: 'Edit' })}
                    onClick={this.#edit}
                />
            </div>
        );
    };

    #renderRow = (): ReactElement => {
        const { Handle, DurationInput, Icon, Toggler, Labels, Actions } = this;

        return (
            <Fragment>
                <Handle />

                <Toggler labeled />

                <Icon />

                <Labels types="inline" />

                <DurationInput label="none" />

                <Actions />
            </Fragment>
        );
    };
    #renderCard = (): ReactElement => {
        const { Handle, DurationInput, Icon, Toggler, Actions, Labels } = this;
        const { formatMessage } = this.props.intl;

        return (
            <Fragment>
                <section className={css.top}>
                    <Handle />

                    <Icon />

                    <Labels types="below" />

                    <Toggler />
                </section>

                <section className={css.bottom}>
                    <DurationInput label="top" />

                    <div className={css.vr} />

                    <CarbonFormField
                        className={css.actionsWrapper}
                        labelText={formatMessage({ defaultMessage: 'Actions' })}
                    >
                        <Actions />
                    </CarbonFormField>
                </section>
            </Fragment>
        );
    };

    render() {
        const { enabled, dndRootProps, className, style, layout, children } = this.props;

        let content: ReactNode;
        const classNames: ClassValue[] = [!enabled && css.disabledRow, className];
        switch (layout) {
            case 'card':
                content = this.#renderCard();
                classNames.push(css.card);
                break;

            case 'row':
                content = this.#renderRow();
                classNames.push(css.row);
                break;

            default:
                assertUnreachable(layout, 'layout');
        }

        return (
            <div {...dndRootProps} style={style} className={cn(classNames)}>
                {content}
                {children}
            </div>
        );
    }
}
export function SceneOverviewRow(props: SceneOverviewRowProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

export interface SceneOverviewRowSkeletonProps extends HTMLAttributes<HTMLDivElement> {
    rowCount?: number;
}
export function SceneOverviewRowSkeleton(props: SceneOverviewRowSkeletonProps) {
    const { rowCount, className, ...rest } = props;

    if (rowCount != null && rowCount > 1) {
        const opacityBase = 0.6;
        const opacityStepSize = opacityBase / rowCount;
        const items: ReactNode[] = Array.from({ length: rowCount }, (_, i) => (
            <SceneOverviewRowSkeleton key={i} style={{ opacity: opacityBase - opacityStepSize * i }} />
        ));

        return <div {...rest} children={items} className={cn(css.skeletonsGroup, className)} />;
    }

    return (
        <div {...rest} role="listitem" aria-hidden="true" className={cn(css.skeleton, className)}>
            <div className={css.a} />
            <div className={css.b1} />
            <div className={css.b2} />
            <div className={css.c} />
            <div className={css.d} />
        </div>
    );
}
