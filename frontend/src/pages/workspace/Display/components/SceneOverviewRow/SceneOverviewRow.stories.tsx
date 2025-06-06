import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { ScenePreview } from '../images/preview';
import { SceneOverviewRow as Component, type SceneOverviewRowProps } from './SceneOverviewRow';

export default {
    title: 'display/components/SceneOverviewRow',
    component: SceneOverviewRow,
    args: {
        id: '1',

        enabled: true,
        onToggle: action('onToggle'),

        duration: '10',
        onDurationChange: action('onDurationChange'),

        onEdit: action('onEdit'),
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
                preview={<ScenePreview kind={pb.SceneKind.combined} />}
                title="Combined Scene"
                description="Clock, Clock, Weather, Ticker (BTC-USD)"
            />
            <Component
                {...args}
                preview={<ScenePreview kind={pb.SceneKind.image} />}
                title="Image"
                description="Your Image"
            />
            <Component
                {...args}
                preview={<ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.analog_rect} />}
                title="Clock – Analog Rectangular"
                description="Horizontal analog layout in a rectangular frame"
            />
            <Component
                {...args}
                preview={<ScenePreview kind={pb.SceneKind.ticker} variant={pb.SceneVariantTicker.candle} />}
                title="Ticker: Big Price"
                description="BTC-USD"
            />
            <Component
                {...args}
                preview={<ScenePreview kind={pb.SceneKind.pool} />}
                title="Braiins Pool Stats"
                description="account.name"
            />
            <Component
                {...args}
                enabled={false}
                preview={<ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.digital_flip} />}
                title="Clock – Flip"
                description="Flip-style digital clock with adjustable font weight"
            />
        </div>
    );
}
