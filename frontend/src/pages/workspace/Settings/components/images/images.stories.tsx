import type { Meta } from '@storybook/react';
import { ClockScenePreview } from './index';

export default {
    title: 'settings/components/Images',
} satisfies Meta;

export function Images() {
    return (
        <div style={{ display: 'flex', flexFlow: 'column nowrap', gap: 8, padding: 8, width: 600 }}>
            <ClockScenePreview variant="analog-rect" />
            <ClockScenePreview variant="analog-round" />
            <ClockScenePreview variant="digital-flip" />
            <ClockScenePreview variant="digital-plain" />
        </div>
    );
}
