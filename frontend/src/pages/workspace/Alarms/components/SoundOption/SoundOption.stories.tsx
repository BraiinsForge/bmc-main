// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
