import type { CSSProperties } from 'react';
import { action } from 'storybook/actions';
import styled from '@emotion/styled';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { ScenePreview } from '../images/preview';
import {
    SceneOverviewRow as Component,
    type SceneOverviewRowProps,
    SceneOverviewRowSkeleton,
} from './SceneOverviewRow';

const clockManifest = pb.create(pb.WidgetManifestSchema, {
    uid: 'manifest-clock',
    name: 'Clock',
    description: 'Displays the current time.',
    version: '0.0.0',
    supportedSizes: [pb.WidgetSize.FULL],
    params: [],
});

export default {
    title: 'Display/Components/SceneOverviewRow',
    component: Component,
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
        type: { night: true },
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
        <Base>
            {layouts.map(([layout, styles]) => (
                <Group key={layout} style={styles}>
                    <Component
                        {...args}
                        icon={<ScenePreview kind="combined" />}
                        title="Combined Scene"
                        type={{ night: true }}
                        description="Clock, Clock, Weather, Ticker (BTC-USD)"
                        layout={layout}
                    />
                    <Component
                        {...args}
                        icon={<ScenePreview kind={{ manifest: clockManifest }} />}
                        title="Clock"
                        description="Displays the current time."
                        layout={layout}
                    />
                    <Component
                        {...args}
                        icon={<ScenePreview kind={null} />}
                        title="N/A"
                        description="No preview — ScenePreview returns null when kind is missing."
                        layout={layout}
                    />
                </Group>
            ))}
        </Base>
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
