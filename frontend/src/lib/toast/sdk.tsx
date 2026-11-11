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

import { useIntl } from 'react-intl';
import { Toaster as SonnerToaster, toast as sonnerToast } from 'sonner';
import { v4 as UUIDv4 } from 'uuid';
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
    const { formatMessage } = useIntl();
    return (
        <SonnerToaster
            // Alt+T would focus the toasts, which leave on their own within seconds and hold no action;
            // sonner does not stop the browser's own Alt+T, and on a Mac it is a character typed in a field.
            hotkey={[]}
            customAriaLabel={formatMessage({
                defaultMessage: 'Notifications',
                description: "Screen-reader name of the corner region where the app's brief notifications appear",
            })}
            {...props}
            toastOptions={{ ...props.toastOptions, className }}
        />
    );
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
    if (heldErrors) {
        const id = extra?.id ?? UUIDv4();
        heldErrors.push({ message, extra: { ...extra, id } });
        return id;
    }
    return show('error', message, extra);
}

// A session check that never answers must not keep errors from the user.
const HELD_ERRORS_MAX_MS = 10_000;
let heldErrors: null | Array<{ message: string; extra: Extra }> = null;
let heldErrorsLimit: undefined | ReturnType<typeof setTimeout>;
/** Keep error toasts back until `releaseErrors` says whether they still apply, or `HELD_ERRORS_MAX_MS` pass. */
function holdErrors(): void {
    if (heldErrors) return;
    heldErrors = [];
    heldErrorsLimit = setTimeout(() => releaseErrors(true), HELD_ERRORS_MAX_MS);
}
/** Show the error toasts held back, or drop them. */
function releaseErrors(apply: boolean): void {
    clearTimeout(heldErrorsLimit);
    const held = heldErrors ?? [];
    heldErrors = null;
    if (apply) for (const { message, extra } of held) show('error', message, extra);
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
    holdErrors,
    releaseErrors,
};
