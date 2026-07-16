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

import { createContext } from 'react';
import type * as pb from '@/proto';

export interface ConfirmationDescriptor {
    title?: string;
    message: NonNullable<ReactNode>;

    confirmLabel?: string;
    cancelLabel?: string;

    size?: 'xs' | 'sm' | 'lg';
    danger?: boolean;
}
export interface AppContextType {
    confirm(d: ConfirmationDescriptor): Promise<boolean>;
    device: {
        sound: {
            play(sound: pb.SoundInfo, signal: AbortSignal): Promise<void>;
            stop(): void;
            currentlyPlaying: null | pb.SoundInfo;
        };
    };
}

export const getAppContextDefault = (): AppContextType => ({
    confirm: () => Promise.resolve(false),
    device: {
        sound: {
            currentlyPlaying: null,
            stop() {},
            play(): Promise<void> {
                return Promise.reject(new Error('Not implemented'));
            },
        },
    },
});

export default createContext<AppContextType>(getAppContextDefault());
