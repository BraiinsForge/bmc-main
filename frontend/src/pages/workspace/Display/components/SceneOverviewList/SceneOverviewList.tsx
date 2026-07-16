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

import { Component, useRef, type Ref } from 'react';
import { useSize } from '@/lib/react';
import { useIntl, type IntlShape } from 'react-intl';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { ScenePreview } from '../images';
import { type RenderSortableListItemProps, Sortable } from '@/components';
import { SceneOverviewRow, SceneOverviewRowSkeleton } from '../SceneOverviewRow';
import { WidgetName } from '../WidgetName';

// Styles
import cn from 'clsx';
import css from './SceneOverviewList.scss';

export interface SceneOverviewListProps {
    scenes: pb.Scene[];
    manifests: pb.ManifestLookup;
    onMove(scenes: pb.Scene[], move: { id: string; from: number; into: number }): void;
    onEdit(id: string): void;
    onClone(id: string): void;
    onDelete(id: string): void;
    onToggle(id: string, value: boolean): void;

    cycleEnabled: boolean;
    cycleDefaultDuration: number;
    onDurationChange(id: string, value: string): void;
}
interface Props extends SceneOverviewListProps {
    intl: IntlShape;
    sizeRef: Ref<HTMLDivElement>;
    useCardLayout: boolean;
}

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

    componentWillUnmount = () => pb.abort.all(this);

    #renderItem = (
        props: RenderSortableListItemProps<pb.Scene>,
        firstEnabledSceneID: Maybe<pb.Scene['id']>,
        locked: boolean,
    ) => {
        const {
            cycleEnabled,
            cycleDefaultDuration,
            onEdit,
            onToggle,
            onClone,
            onDelete,
            onDurationChange,
            intl,
            useCardLayout,
            manifests,
        } = this.props;
        const { item, state, rootProps, dragHandleProps } = props;

        const title = pb.sceneTitle(intl, item, manifests) || 'N/A';
        const description = pb.sceneDescription(item, manifests) || '';

        const previewKind: Maybe<'combined' | { manifest?: pb.WidgetManifest }> = (() => {
            switch (item.kind.case) {
                case undefined:
                    return null;
                case 'combined':
                    return 'combined';
                case 'fullscreen': {
                    const fw = item.kind.value.widget;
                    const manifest = fw?.config?.widgetUid ? manifests.get(fw.config.widgetUid) : undefined;
                    return { manifest };
                }
                default:
                    return null;
            }
        })();

        // Fullscreen scenes show the widget's name + optional grayed subname.
        const fullscreenManifest = previewKind && previewKind !== 'combined' ? previewKind.manifest : undefined;
        const titleNode = fullscreenManifest?.subname ? (
            <WidgetName name={title} subname={fullscreenManifest.subname} />
        ) : (
            title
        );

        const isNightModeWidget: boolean = firstEnabledSceneID === item.id;

        return (
            <SceneOverviewRow
                id={item.id}
                locked={locked}
                className={cn(
                    css.line,
                    state.isDragging && css.dragged,
                    state.isOver && !state.isDragging && css.dropTarget,
                )}
                layout={useCardLayout ? 'card' : 'row'}
                enabled={item.enabled}
                icon={<ScenePreview kind={previewKind} />}
                title={titleNode}
                type={{ night: isNightModeWidget }}
                description={description}
                cycleEnabled={cycleEnabled}
                cycleDurationValue={item.cycleDurationSec}
                cycleDurationDefault={cycleDefaultDuration}
                // Handlers
                onEdit={onEdit}
                onClone={onClone}
                onToggle={onToggle}
                onDelete={onDelete}
                onDurationChange={onDurationChange}
                // DnD
                dndRootProps={rootProps}
                dndDragHandleProps={dragHandleProps}
            />
        );
    };

    render() {
        const { scenes, onMove, intl, sizeRef } = this.props;

        const firstEnabledSceneID = scenes.find(x => x.enabled)?.id;
        // A placeholder's indices don't match the backend yet — lock the whole
        // list (controls + drag) until the reload settles.
        const settling = scenes.some(x => pb.isOptimisticSceneId(x.id));
        if (!scenes.length) {
            return (
                <div className={css.placeholder}>
                    <SceneOverviewRowSkeleton rowCount={3} className={css.skeleton} />
                    <h1
                        className={css.title}
                        children={intl.formatMessage({ defaultMessage: 'No “Display widget” yet' })}
                    />
                </div>
            );
        }

        return (
            <Sortable<pb.Scene>
                wrapperRef={sizeRef}
                className={css.list}
                items={scenes}
                onChange={onMove}
                isItemDisabled={() => settling}
                renderItem={x => this.#renderItem(x, firstEnabledSceneID, settling)}
            />
        );
    }
}

export function SceneOverviewList(props: SceneOverviewListProps) {
    const intl = useIntl();

    const sizeRef = useRef<HTMLDivElement>(null);
    const size = useSize(sizeRef, 300);
    const useCardLayout: boolean = !!size && size.width <= 800;

    return <View {...props} intl={intl} sizeRef={sizeRef} useCardLayout={useCardLayout} />;
}
