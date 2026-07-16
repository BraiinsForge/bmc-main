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
    const duration = extra?.duration;
    return sonnerToast.custom(id => <Toast id={id} kind={kind} title={extra?.title} message={message} />, {
        ...extra,
        duration: duration != null && Number.isFinite(duration) ? duration * 1e3 : 3e3,
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
