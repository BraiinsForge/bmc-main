import { createContext } from 'react';

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
}

export const getAppContextDefault = (): AppContextType => ({
    notify: Object.assign(() => {}, { clear() {} }),
    confirm: () => Promise.resolve(false),
});

export default createContext<AppContextType>(getAppContextDefault());
