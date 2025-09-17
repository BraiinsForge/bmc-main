import type { Meta } from '@storybook/react';

import { SoundOption as Component, type SoundOptionProps } from './SoundOption';
import AppContext, { type AppContextType, getAppContextDefault } from '@/context';
import * as get from '@/mocks';
import * as pb from '@/proto';

export default {
    title: 'Alarms/components/SoundOption',
    component: Component,
} satisfies Meta<SoundOptionProps>;

const sounds: pb.SoundInfo[] = get.arrayOf<pb.SoundInfo>(5, () =>
    pb.create(pb.SoundInfoSchema, { id: get.uuid(), name: get.hostname(2, '') }),
);

const appContextValue: AppContextType = {
    ...getAppContextDefault(),
    device: {
        sound: {
            currentlyPlaying: null,
            async play(sound: pb.SoundInfo, signal: AbortSignal): Promise<void> {
                console.log('play', sound, signal);
                await new Promise(resolve => setTimeout(resolve, 1000));
                console.log('play done', sound, signal);
                return Promise.resolve();
            },
            stop(): void {
                console.log('stop');
            },
        },
    },
};

export function SoundOption() {
    return (
        <AppContext value={appContextValue}>
            <div
                style={{
                    display: 'inline flex',
                    flexDirection: 'column',
                    gap: 8,
                    margin: 16,
                }}
                children={sounds.map(x => (
                    <Component
                        key={x.id}
                        sound={x}
                        style={{
                            padding: '0.5rem 1rem',
                            backgroundColor: 'var(--cds-layer-01)',
                        }}
                    />
                ))}
            />
        </AppContext>
    );
}
