import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

// App
import type * as pb from '@/proto';
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
    setScenes(scenes: pb.Scene[]): void;
}
interface Props extends SceneOverviewListProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

    #noop = (): void => {
        this.context.notify('error', 'Not implemented!');
    };
    #renderItem: SortableProps<pb.Scene>['renderItem'] = props => {
        const { item, state, rootProps, dragHandleProps } = props;
        return (
            <SceneOverviewRow
                id={item.id}
                className={cn(css.line, state.isDragging && css.dragged)}
                enabled={item.enabled}
                preview={<ScenePreview kind={item.kind} variant={item.variant} />}
                title={item.title}
                description={item.description}
                duration={item.durationSeconds}
                // Handlers
                onEdit={this.#noop}
                onToggle={this.#noop}
                onDelete={this.#noop}
                onDurationChange={this.#noop}
                // DnD
                dndRootProps={rootProps}
                dndDragHandleProps={dragHandleProps}
            />
        );
    };

    render() {
        const { scenes, setScenes } = this.props;

        return (
            <Sortable<pb.Scene>
                className={css.list}
                items={scenes}
                onChange={setScenes}
                renderItem={this.#renderItem}
            />
        );
    }
}

export function SceneOverviewList(props: SceneOverviewListProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
