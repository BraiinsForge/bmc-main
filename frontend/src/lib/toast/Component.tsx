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

import { Fragment, useCallback } from 'react';
import { toast, type ToastProps } from './sdk';

// Components
import { Html } from '@/components';
import { ToastNotification } from '@carbon/react';

// Styles
import css from './Component.scss';

export const className = css.item;

/** A fully custom toast that still maintains the animations and interactions. */
export function Toast(props: ToastProps) {
    const { id, title, message, kind } = props;

    const dismiss = useCallback(() => toast.dismiss(id), [id]);

    return (
        <ToastNotification
            key={id}
            kind={kind}
            title={title}
            children={<Fragment>{typeof message === 'string' ? <Html children={message} /> : message}</Fragment>}
            onClose={dismiss}
        />
    );
}
