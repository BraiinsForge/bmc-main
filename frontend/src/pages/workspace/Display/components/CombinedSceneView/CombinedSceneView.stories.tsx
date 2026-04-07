import { useState, useCallback } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import * as fn from '../../fn';
import { CombinedSceneView as View, type CombinedSceneViewProps } from './CombinedSceneView';

export default {
    title: 'Display/Components/CombinedSceneView',
    component: View,
} satisfies Meta<CombinedSceneViewProps>;

const MANIFEST_UID = 'storybook-widget-uid';

const manifests: pb.ManifestLookup = new Map([
    [
        MANIFEST_UID,
        pb.create(pb.WidgetManifestSchema, {
            uid: MANIFEST_UID,
            name: 'Demo Widget',
            description: 'Storybook placeholder.',
            version: '0.0.0',
            supportedSizes: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE, pb.WidgetSize.FULL],
            params: [],
        }),
    ],
]);

function widget(id: string, position: pb.WidgetPosition, size: pb.WidgetSize): pb.Widget {
    return pb.create(pb.WidgetSchema, {
        id,
        position,
        size,
        config: pb.create(pb.WidgetConfigSchema, { widgetUid: MANIFEST_UID, params: {} }),
    });
}

const test1: pb.Widget[] = [
    widget('1', fn.pos(0, 0), pb.WidgetSize.LARGE),
    widget('2', fn.pos(0, 2), pb.WidgetSize.SMALL),
    widget('3', fn.pos(1, 2), pb.WidgetSize.MEDIUM),
];
const test2: pb.Widget[] = [widget('1', fn.pos(0, 0), pb.WidgetSize.LARGE)];

function Demo(props: Pick<CombinedSceneViewProps, 'widgets'>) {
    const [widgets, setWidgets] = useState<pb.Widget[]>(props.widgets);

    const onWidgetAdd: CombinedSceneViewProps['onWidgetAdd'] = useCallback((pos: pb.WidgetPosition) => {
        action('onWidgetAdd')(pos);
    }, []);
    const onWidgetMove: CombinedSceneViewProps['onWidgetMove'] = useCallback(
        (src: pb.Widget, tgt: fn.Located): void => {
            action('onWidgetMove')(src, tgt);
            const srcSlot = { position: src.position, size: src.size };
            const tgtSlot = { position: tgt.position, size: tgt.size };
            setWidgets(s =>
                s.map(x => {
                    if (x.id === src.id) return { ...x, ...tgtSlot } as pb.Widget;
                    if (x.id === tgt.id) return { ...x, ...srcSlot } as pb.Widget;
                    return x;
                }),
            );
        },
        [],
    );
    const onWidgetEdit: CombinedSceneViewProps['onWidgetEdit'] = useCallback((id: string) => {
        action('onWidgetEdit')(id);
    }, []);
    const onWidgetRemove: CombinedSceneViewProps['onWidgetRemove'] = useCallback((id: string) => {
        setWidgets(s => s.filter(x => x.id !== id));
    }, []);

    return (
        <View
            widgets={widgets}
            manifests={manifests}
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
        <div style={{ display: 'inline-flex', flexDirection: 'column', gap: 16 }}>
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
