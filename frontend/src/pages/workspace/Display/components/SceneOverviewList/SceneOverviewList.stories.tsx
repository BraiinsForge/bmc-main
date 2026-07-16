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

import { useState } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { SceneOverviewList as Component, type SceneOverviewListProps } from './SceneOverviewList';

export default {
    title: 'Display/Components/SceneOverviewList',
    component: Component,
} satisfies Meta<SceneOverviewListProps>;

const CLOCK_UID = 'manifest-clock';
const TICKER_UID = 'manifest-ticker';
const UNKNOWN_UID = 'manifest-unknown';

function manifest(uid: string, name: string, description: string): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, {
        uid,
        name,
        description,
        version: '0.0.0',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [],
    });
}

const manifests: pb.ManifestLookup = new Map([
    [CLOCK_UID, manifest(CLOCK_UID, 'Clock', 'Displays the current time.')],
    [TICKER_UID, manifest(TICKER_UID, 'Ticker', 'Displays a price ticker.')],
]);

function fullscreenScene(id: string, widgetUid: string, cycleDurationSec: number, enabled: boolean): pb.Scene {
    return pb.create(pb.SceneSchema, {
        id,
        enabled,
        cycleDurationSec,
        kind: {
            case: 'fullscreen',
            value: pb.create(pb.Scene_FullscreenSchema, {
                widget: pb.create(pb.WidgetSchema, {
                    id: `widget-${id}`,
                    size: pb.WidgetSize.FULL,
                    position: pb.create(pb.WidgetPositionSchema, { row: 0, col: 0 }),
                    config: pb.create(pb.WidgetConfigSchema, { widgetUid, params: {} }),
                }),
            }),
        },
    });
}

const initialState: pb.Scene[] = [
    fullscreenScene('0', CLOCK_UID, 10, true),
    fullscreenScene('1', TICKER_UID, 11, true),
    fullscreenScene('2', CLOCK_UID, 30, false),
];

function buildView(scenes: pb.Scene[], lookup: pb.ManifestLookup) {
    function View() {
        const [s, setScenes] = useState<pb.Scene[]>(scenes);
        return (
            <Component
                scenes={s}
                manifests={lookup}
                onMove={next => setScenes(next)}
                onEdit={action('onEdit')}
                onClone={action('onClone')}
                onDelete={action('onDelete')}
                onToggle={action('onToggle')}
                onDurationChange={action('onDurationChange')}
                cycleEnabled
                cycleDefaultDuration={30}
            />
        );
    }
    return View;
}

export const SceneOverviewList = buildView(initialState, manifests);

const fallbackState: pb.Scene[] = [
    fullscreenScene('0', CLOCK_UID, 10, true),
    fullscreenScene('1', UNKNOWN_UID, 12, true),
];
export const MissingManifestFallback = buildView(fallbackState, manifests);
