import { useState } from 'react';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { SceneOverviewList as Component, type SceneOverviewListProps } from './SceneOverviewList';

export default {
    title: 'display/components/SceneOverviewList',
    component: Component,
} satisfies Meta<SceneOverviewListProps>;

const initialState: pb.Scene[] = [
    {
        id: 0,
        enabled: true,
        durationSeconds: 10,
        kind: pb.SceneKind.combined,
        title: 'Combined Scene',
        description: 'Clock, Clock, Weather, Ticker (BTC-USD)',
    } satisfies pb.Scene,
    {
        id: 1,
        durationSeconds: 11,
        enabled: true,
        kind: pb.SceneKind.image,
        title: 'Image',
        description: 'Your Image',
    } satisfies pb.Scene,
    {
        id: 2,
        enabled: true,
        durationSeconds: 11,
        kind: pb.SceneKind.clock,
        variant: pb.SceneVariantClock.analog_rect,
        title: 'Clock – Analog Rectangular',
        description: 'Horizontal analog layout in a rectangular frame',
    } satisfies pb.Scene,
    {
        id: 3,
        enabled: true,
        durationSeconds: 13,
        kind: pb.SceneKind.ticker,
        variant: pb.SceneVariantTicker.candle,
        title: 'Ticker: Big Price',
        description: 'BTC-USD',
    } satisfies pb.Scene,
    {
        id: 4,
        enabled: true,
        durationSeconds: 14,
        kind: pb.SceneKind.pool,
        title: 'Braiins Pool Stats',
        description: 'account.name',
    } satisfies pb.Scene,
    {
        id: 5,
        durationSeconds: 15,
        enabled: false,
        kind: pb.SceneKind.clock,
        variant: pb.SceneVariantClock.digital_flip,
        title: 'Clock – Flip',
        description: 'Flip-style digital clock with adjustable font weight',
    } satisfies pb.Scene,
];

function Demo() {
    const [scenes, setScenes] = useState<pb.Scene[]>(initialState);
    return <Component scenes={scenes} setScenes={setScenes} />;
}

export function SceneOverviewList() {
    return <Demo />;
}
