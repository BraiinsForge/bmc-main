import { Component, useRef, type Ref } from 'react';
import { useSize } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';
import { useIntl, type IntlShape } from 'react-intl';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { ScenePreview } from '../images';
import { type RenderSortableListItemProps, Sortable } from '@/components';
import { SceneOverviewRow, SceneOverviewRowSkeleton } from '../SceneOverviewRow';

// Styles
import cn from 'clsx';
import css from './SceneOverviewList.scss';

export interface SceneOverviewListProps {
    scenes: pb.Scene[];
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

    #isRemote = (x: Maybe<ProtoOneofCase<pb.WidgetKind['value']>>): boolean => {
        return x === 'remoteWidget' || x === 'remoteImage';
    };
    #renderItem = (props: RenderSortableListItemProps<pb.Scene>, firstEnabledSceneID: Maybe<pb.Scene['id']>) => {
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
        } = this.props;
        const { item, state, rootProps, dragHandleProps } = props;

        let title: string = 'N/A';
        let description: string = '';
        switch (item.kind.case) {
            case undefined:
                break;

            case 'combined':
                title = intl.formatMessage({ defaultMessage: 'Combined Scene' });
                description = pb.sceneDescription(intl, item.kind.value.widgets) || '';
                break;

            case 'fullscreen':
                title =
                    item.kind.value.widget?.kind?.value.case === 'remoteWidget'
                        ? item.kind.value.widget.kind.value.value.name
                        : (pb.sceneTitle(intl, item.kind.value.widget?.kind?.value.case) ?? 'N/A');
                title ||= 'N/A';
                description = pb.sceneDescription(intl, item.kind.value.widget) || '';
                break;

            default:
                assertUnreachable(item.kind, 'scene kind');
        }

        const $kind = item.kind;
        const isNightModeWidget: boolean = firstEnabledSceneID === item.id;
        const isRemoteWidgetOrHasOneInside: boolean =
            ($kind.case === 'fullscreen' && this.#isRemote($kind.value?.widget?.kind?.value.case)) ||
            ($kind.case === 'combined' && $kind.value.widgets.some(x => this.#isRemote(x.kind?.value.case)));
        const isLocalWidgetOrHasOneInside: boolean =
            ($kind.case === 'fullscreen' && !this.#isRemote($kind.value?.widget?.kind?.value.case)) ||
            ($kind.case === 'combined' && !$kind.value.widgets.some(x => this.#isRemote(x.kind?.value.case)));

        return (
            <SceneOverviewRow
                id={item.id}
                className={cn(
                    css.line,
                    state.isDragging && css.dragged,
                    state.isOver && !state.isDragging && css.dropTarget,
                )}
                layout={useCardLayout ? 'card' : 'row'}
                enabled={item.enabled}
                icon={
                    <ScenePreview
                        kind={item.kind.case === 'fullscreen' ? item.kind.value?.widget?.kind?.value : 'combined'}
                    />
                }
                title={title}
                type={{
                    night: isNightModeWidget,
                    cloud: isRemoteWidgetOrHasOneInside,
                    local: isLocalWidgetOrHasOneInside,
                }}
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
                renderItem={x => this.#renderItem(x, firstEnabledSceneID)}
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
