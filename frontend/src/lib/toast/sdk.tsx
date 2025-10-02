import { Toaster as SonnerToaster, toast as sonnerToast } from 'sonner';
import type { ToasterProps, ExternalToast } from 'sonner';

import { className, Toast } from './Component';

export type { ToasterProps };
export type ToastID = string | number;
export interface ToastProps {
    id: ToastID;
    kind: 'success' | 'info' | 'warning' | 'error';
    title?: string;
    message: string;
}

export function Toaster(props: ToasterProps) {
    return <SonnerToaster {...props} toastOptions={{ ...props.toastOptions, className }} />;
}

type Extra = ExternalToast & { title?: string };
function show(kind: ToastProps['kind'], message: ToastProps['message'], extra?: Extra): ToastID {
    return sonnerToast.custom(id => <Toast id={id} kind={kind} title={extra?.title} message={message} />, {
        ...extra,
        duration: extra?.duration ? extra.duration * 1e3 : undefined,
    });
}
function success(message: ToastProps['message'], extra?: Extra): ToastID {
    return show('success', message, extra);
}
function error(message: ToastProps['message'], extra?: Extra): ToastID {
    return show('error', message, extra);
}

/** Accepts the id of the toast to dismiss, `null` to dismiss all toasts */
function dismiss(id: null | string | number): void {
    sonnerToast.dismiss(id == null ? undefined : id);
}

export const toast = {
    show,
    success,
    error,

    dismiss,
};
