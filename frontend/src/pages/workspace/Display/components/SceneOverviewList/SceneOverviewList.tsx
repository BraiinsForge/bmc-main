import { Component } from 'react';
import { assertUnreachable } from '@/lib/ts.ts';
import { useIntl, type IntlShape } from 'react-intl';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Sortable, type SortableProps } from '@/components';
import { SceneOverviewRow } from '../SceneOverviewRow';
import { ScenePreview } from '../images';

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

    defaultSceneDuration: number;
    onDurationChange(id: string, value: string): void;
}
interface Props extends SceneOverviewListProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

    componentWillUnmount = () => pb.abort.all(this);

    #renderItem: SortableProps<pb.Scene>['renderItem'] = props => {
        const { defaultSceneDuration, onEdit, onToggle, onClone, onDelete, onDurationChange, intl } = this.props;
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
                title = pb.sceneTitle(intl, item.kind.value.widget?.kind?.value.case) ?? 'N/A';
                description = pb.sceneDescription(intl, item.kind.value.widget) || '';
                break;

            default:
                assertUnreachable(item.kind, 'scene kind');
        }

        return (
            <SceneOverviewRow
                id={item.id}
                className={cn(css.line, state.isDragging && css.dragged)}
                enabled={item.enabled}
                preview={
                    <ScenePreview
                        kind={item.kind.case === 'fullscreen' ? item.kind.value?.widget?.kind?.value : 'combined'}
                    />
                }
                title={title}
                description={description}
                duration={item.cycleDurationSec ?? defaultSceneDuration}
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
        const { scenes, onMove } = this.props;

        return (
            <Sortable<pb.Scene> className={css.list} items={scenes} onChange={onMove} renderItem={this.#renderItem} />
        );
    }
}

export function SceneOverviewList(props: SceneOverviewListProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
