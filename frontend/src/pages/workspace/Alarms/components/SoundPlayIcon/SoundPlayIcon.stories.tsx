import type { Meta } from '@storybook/react';
import { SoundPlayIcon as Component } from './SoundPlayIcon';

export default {
    title: 'Alarms/components/SoundPlayIcon',
    component: Component,
} satisfies Meta;

export function SoundPlayIcon() {
    return (
        <div style={{ display: 'inline flex', flexDirection: 'column', gap: 8, margin: 16 }}>
            <Component isPlaying={false} />
            <Component isPlaying />
        </div>
    );
}
