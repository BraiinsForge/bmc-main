import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import colors from '@/styles/colors';

import * as pb from '@/proto';
import { ScenePreview } from '../images/preview';
import {
    SceneOverviewRow as Component,
    type SceneOverviewRowProps,
    SceneOverviewRowSkeleton,
} from './SceneOverviewRow';

export default {
    title: 'display/components/SceneOverviewRow',
    component: SceneOverviewRow,
    args: {
        id: '1',

        enabled: true,
        onToggle: action('onToggle'),

        duration: 10,
        durationDefault: 11,
        onDurationChange: action('onDurationChange'),

        onEdit: action('onEdit'),
        onClone: action('onClone'),
        onDelete: action('onDelete'),

        preview: null,
        title: '',
        description: '',
    } satisfies SceneOverviewRowProps,
} satisfies Meta<SceneOverviewRowProps>;

export function SceneOverviewRow(args: SceneOverviewRowProps) {
    return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
            <Component
                {...args}
                preview={<ScenePreview kind="combined" />}
                title="Combined Scene"
                tag={{
                    type: 'blue',
                    text: 'Night Mode',
                    style: {
                        color: colors.blue20,
                        backgroundColor: colors.blue90,
                    },
                }}
                description="Clock, Clock, Weather, Ticker (BTC-USD)"
            />
            {/* <Component {...args} preview={<ScenePreview kind={pb.SceneKind.image} />} title="Image" description="Your Image" />*/}
            <Component
                {...args}
                preview={
                    <ScenePreview
                        kind={{
                            case: 'clock',
                            value: pb.create(pb.ClockWidgetSchema, {
                                clockStyle: pb.ClockWidget_ClockStyle.ANALOG_RECT,
                            }),
                        }}
                    />
                }
                title="Clock – Analog Rectangular"
                description="Horizontal analog layout in a rectangular frame"
            />
            {/* <Component {...args} preview={<ScenePreview kind={pb.SceneKind.ticker} variant={pb.SceneVariantTicker.candle} />} title="Ticker: Big Price" description="BTC-USD" /> */}
            {/* <Component {...args} preview={<ScenePreview kind={pb.SceneKind.pool} />} title="Braiins Pool Stats" description="account.name" /> */}
            {/* <Component {...args} enabled={false} preview={<ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.digital_flip} />} title="Clock – Flip" description="Flip-style digital clock with adjustable font weight" /> */}
        </div>
    );
}
SceneOverviewRow.storyName = 'SceneOverviewRow';

export function Skeleton() {
    return (
        <div style={{ backgroundColor: 'var(--cds-background)' }}>
            <SceneOverviewRowSkeleton rowCount={6} />
        </div>
    );
}
