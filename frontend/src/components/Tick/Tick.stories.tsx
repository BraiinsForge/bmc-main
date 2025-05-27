import type { Meta } from '@storybook/react';
import { Progressbar } from '../Progressbar';
import * as gen from '@/mocks';

import { Tick as Component, type TickProps } from './Tick';

export default {
    title: 'components/Tick',
    component: Component,
    args: {
        intervalMs: 1e3,
    } satisfies TickProps,
} satisfies Meta<TickProps>;

const startTime = gen.timestamp(0);

export function Tick(args: TickProps) {
    return (
        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 16 }}>
            <Component {...args} render={value => <div children={value} />} />
            <Component {...args} render={value => <Progressbar values={[{ value: value - startTime }]} />} />
        </div>
    );
}
