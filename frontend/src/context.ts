import { createContext } from 'react';

export type NotificationType = 'info' | 'warning' | 'error' | 'success';
export type NotificationMessage = NonNullable<ReactNode>;
export interface Notify {
    (type: NotificationType, message: NotificationMessage, timeoutSeconds?: number): void;
    clear(): void;
}

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
