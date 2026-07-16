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
