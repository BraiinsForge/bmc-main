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

        cycleEnabled: true,
        cycleDurationValue: 10,
        cycleDurationDefault: 11,
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
            <Component
                {...args}
                preview={<ScenePreview kind={{ case: 'tickerBtc', value: pb.create(pb.TickerBtcWidgetSchema, {}) }} />}
                title="Ticker: BTC Price"
                description="Exsul potuss, tanquam velox extum."
            />
            <Component
                {...args}
                preview={
                    <ScenePreview kind={{ case: 'blockHeight', value: pb.create(pb.BlockHeightWidgetSchema, {}) }} />
                }
                title="Block Height"
                description="Teachers, winds, and special saints will always protect them."
            />
            {/* <Component {...args} preview={<ScenePreview kind={pb.SceneKind.pool} />} title="Braiins Pool Stats" description="account.name" /> */}
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
