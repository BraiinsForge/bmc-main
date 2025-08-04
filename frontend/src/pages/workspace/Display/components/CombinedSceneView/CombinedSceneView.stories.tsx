import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import * as fn from '../../fn';
import { CombinedSceneView as View, type CombinedSceneViewProps } from './CombinedSceneView';

export default {
    title: 'display/components/CombinedSceneView',
    component: View,
};

const test1: pb.Widget[] = [
    {
        $typeName: 'braiins.bmc.web.Widget',
        id: '1',
        position: fn.pos(0, 0),
        size: pb.WidgetSize.LARGE,
    },
    {
        $typeName: 'braiins.bmc.web.Widget',
        id: '2',
        position: fn.pos(0, 2),
        size: pb.WidgetSize.SMALL,
    },
    {
        $typeName: 'braiins.bmc.web.Widget',
        id: '3',
        position: fn.pos(1, 2),
        size: pb.WidgetSize.MEDIUM,
    },
];
const test2: pb.Widget[] = [
    {
        $typeName: 'braiins.bmc.web.Widget',
        id: '1',
        position: fn.pos(0, 0),
        size: pb.WidgetSize.LARGE,
    },
];

function Demo(props: Pick<CombinedSceneViewProps, 'widgets'>) {
    const [widgets, setWidgets] = useState<pb.Widget[]>(props.widgets);

    const onWidgetAdd: CombinedSceneViewProps['onWidgetAdd'] = useCallback((pos: pb.WidgetPosition) => {
        action('onWidgetAdd')(pos);
    }, []);
    const onWidgetMove: CombinedSceneViewProps['onWidgetMove'] = useCallback((src: pb.Widget, tgt: pb.Widget): void => {
        const srcProps = { position: src.position, size: src.size };
        const tgtProps = { position: tgt.position, size: tgt.size };
        setWidgets(s => {
            return s.map(x => {
                if (x.id === src.id) return { ...x, ...tgtProps } as pb.Widget;
                if (x.id === tgt.id) return { ...x, ...srcProps } as pb.Widget;
                return x;
            });
        });
    }, []);
    const onWidgetEdit: CombinedSceneViewProps['onWidgetEdit'] = useCallback((id: string) => {
        action('onWidgetEdit')(id);
    }, []);
    const onWidgetRemove: CombinedSceneViewProps['onWidgetRemove'] = useCallback((id: string) => {
        setWidgets(s => s.filter(x => x.id !== id));
    }, []);

    return (
        <View
            widgets={widgets}
            onWidgetAdd={onWidgetAdd}
            onWidgetMove={onWidgetMove}
            onWidgetEdit={onWidgetEdit}
            onWidgetRemove={onWidgetRemove}
        />
    );
}
function DemoGrid(props: Pick<CombinedSceneViewProps, 'widgets'>) {
    const view = <Demo {...props} />;

    return (
        <div style={{ display: 'inline flex', flexDirection: 'column', gap: 16 }}>
            <div className="ui-box" style={{ width: 1350 }} children={view} />
            <div style={{ display: 'flex', flexFlow: 'row wrap', gap: 16, justifyContent: 'space-between' }}>
                <div className="ui-box" style={{ width: 600 }} children={view} />
                <div className="ui-box" style={{ width: 400 }} children={view} />
                <div className="ui-box" style={{ width: 300 }} children={view} />
            </div>
        </div>
    );
}

export const Demo1 = () => <DemoGrid widgets={test1} />;
export const Demo2 = () => <DemoGrid widgets={test2} />;
