// Copyright (C) 2026  Braiins Systems s.r.o.
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

import type { IntlShape } from 'react-intl';
import type { ActionableNotificationProps } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';

type NotificationKind = NonNullable<ActionableNotificationProps['kind']>;

/** The screen-reader names of a notification's status icon and close button. */
export function notificationLabels(
    formatMessage: IntlShape['formatMessage'],
    kind: NotificationKind,
): { statusIconDescription: string; 'aria-label': string } {
    return {
        statusIconDescription: statusIconDescription(formatMessage, kind),
        'aria-label': formatMessage({
            defaultMessage: 'Close notification',
            description: "Screen-reader name of a notification's close button",
        }),
    };
}

function statusIconDescription(formatMessage: IntlShape['formatMessage'], kind: NotificationKind): string {
    switch (kind) {
        case 'error':
            return formatMessage({
                defaultMessage: 'Error',
                description: "Screen-reader name of an error notification's icon",
            });
        case 'warning':
        case 'warning-alt':
            return formatMessage({
                defaultMessage: 'Warning',
                description: "Screen-reader name of a warning notification's icon",
            });
        case 'info':
        case 'info-square':
            return formatMessage({
                defaultMessage: 'Information',
                description: "Screen-reader name of an informational notification's icon",
            });
        case 'success':
            return formatMessage({
                defaultMessage: 'Success',
                description: "Screen-reader name of a success notification's icon",
            });
        default:
            return assertUnreachable(kind, 'notification kind');
    }
}
