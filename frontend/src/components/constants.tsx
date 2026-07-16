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

import type { MouseEvent, KeyboardEvent, SyntheticEvent, HTMLAttributes } from 'react';

function handleSpaceEnter(event: Maybe<KeyboardEvent>, callback: (e: KeyboardEvent) => void) {
    if (!event?.key) return;
    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        callback(event);
    }
}

const checkbox = (checked: boolean | 'mixed', handler: AnyFunction) => ({
    tabIndex: 0,
    role: 'checkbox',
    'aria-checked': typeof checked === 'boolean' ? checked : checked,
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => handleSpaceEnter(event, handler),
    onClick: handler,
});

const button = (handler: (e: SyntheticEvent) => void, blurOnClick?: boolean) => ({
    tabIndex: 0,
    role: 'button',
    onKeyDown: (event: KeyboardEvent) => handleSpaceEnter(event, handler),
    onClick: (event: MouseEvent) => {
        handler(event);

        // Blur the `activeElement` not to leave the clicked element in focused state
        // (the focus only really makes sense for keyboard navigation)
        // @ts-expect-error: Missing blur method in DOM api
        if (blurOnClick) document.activeElement?.blur();
    },
});

type ArrowHandler = (direction: -1 | 1) => void;
type ToggleHandler = (e: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>) => void;
const listItem = (
    onToggle?: Maybe<ToggleHandler>,
    onArrow?: Maybe<ArrowHandler>,
    labelledby?: string,
    describedby?: string,
) => {
    const r: HTMLAttributes<HTMLElement> = {
        tabIndex: -1,
        role: 'listItem',
        'aria-labelledby': labelledby || undefined,
        'aria-describedby': describedby || undefined,
    };

    if (onToggle || onArrow) {
        r.tabIndex = 0;
        r.onKeyDown = (e: KeyboardEvent<HTMLElement>): void => {
            if (onToggle && (e.key === ' ' || e.key === 'Enter')) {
                e.preventDefault();
                e.stopPropagation();
                onToggle(e);
            }
            if (onArrow && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
                e.preventDefault();
                e.stopPropagation();
                onArrow(e.key === 'ArrowUp' ? -1 : 1);
            }
        };
    }
    if (onToggle) {
        r.onClick = (e: MouseEvent<HTMLElement>): void => {
            onToggle(e);
            e.stopPropagation();
            e.preventDefault();
        };
    }

    return r;
};

export const ARIA = { checkbox, listItem, button };
