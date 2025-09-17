import { createContext } from 'react';
import type * as pb from '@/proto';

export type NotificationType = 'info' | 'warning' | 'error' | 'success';
export type NotificationMessage = NonNullable<ReactNode>;
export type NotificationExtra = {
    // Used for external identification of the notification regardless of the type and/or text.
    // Usefull when newer instances replaces potentially pre-exsting ones.
    id?: number | string;
    timeoutSeconds?: number;
};
export type NotifyFunction = (type: NotificationType, message: NotificationMessage, extra?: NotificationExtra) => void;
export type Notify = NotifyFunction & { clear(id?: NotificationExtra['id']): void };

export interface ConfirmationDescriptor {
    title?: string;
    message: NonNullable<ReactNode>;

    confirmLabel?: string;
    cancelLabel?: string;

    size?: 'xs' | 'sm' | 'lg';
    danger?: boolean;
}
export interface AppContextType {
    notify: Notify;
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
    notify: Object.assign(() => {}, { clear() {} }),
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
