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
