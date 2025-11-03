import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import styled from '@emotion/styled';

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
        onReload: action('onReload'),

        icon: null,
        title: '',
        type: {
            local: true,
            cloud: true,
            night: true,
        },
        description: '',
        layout: 'row',
    } satisfies SceneOverviewRowProps,
} satisfies Meta<SceneOverviewRowProps>;

const Base = styled.div`
    display: flex;
    flex-direction: column;
    gap: 32px;
`;
const Group = styled.div`
    display: flex;
    flex-direction: column;
    gap: 4px;
`;
const layouts: Array<[layout: SceneOverviewRowProps['layout'], containerStyles: CSSProperties]> = [
    ['row', {}],
    ['card', { width: 400, marginInline: 'auto' }],
];

export function SceneOverviewRow(args: SceneOverviewRowProps) {
    return (
        <Base
            children={layouts.map(([layout, styles]) => (
                <Group key={layout} style={styles}>
                    <Component
                        {...args}
                        icon={<ScenePreview kind="combined" />}
                        title="Combined Scene"
                        type={{ local: true, cloud: true, night: true }}
                        description="Clock, Clock, Weather, Ticker (BTC-USD)"
                        layout={layout}
                    />
                    <Component
                        {...args}
                        icon={
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
                        layout={layout}
                    />
                    <Component
                        {...args}
                        icon={
                            <ScenePreview
                                kind={{ case: 'tickerBtc', value: pb.create(pb.TickerBtcWidgetSchema, {}) }}
                            />
                        }
                        title="Ticker: BTC Price"
                        description="Exsul potuss, tanquam velox extum."
                        layout={layout}
                    />
                    <Component
                        {...args}
                        icon={
                            <ScenePreview
                                kind={{ case: 'blockHeight', value: pb.create(pb.BlockHeightWidgetSchema, {}) }}
                            />
                        }
                        title="Block Height"
                        description="Teachers, winds, and special saints will always protect them."
                        layout={layout}
                    />
                </Group>
            ))}
        />
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
