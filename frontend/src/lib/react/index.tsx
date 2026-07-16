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

import type { Component, SyntheticEvent, UIEvent as ReactUIEvent, FocusEvent as ReactFocusEvent } from 'react';

export { HelmetProvider, Helmet } from '@dr.pogodin/react-helmet';

export * from './hooks';
export * from './props';
export * from './render';
export * from './icon';

/**
 * Async version of `setState`.
 * Usefull when you need to wait for the state to be updated before doing something else.
 *
 * @example await setState(this, { … });
 * @example await setState(this, s => ({ … }));
 */
export function setState<State>(
    self: Component<unknown, State>,
    state: Partial<State> | ((prevState: State) => Partial<State>),
): Promise<void> {
    return new Promise(resolve => self.setState(state as State, resolve));
}

export function stopEventPropagation(e: SyntheticEvent | Event) {
    e.stopPropagation();
}
export function blockEvent(e: SyntheticEvent | Event) {
    e.preventDefault();
    e.stopPropagation();
}

export function selfSelect<El extends HTMLInputElement | HTMLTextAreaElement>(e: FocusEvent | ReactFocusEvent<El>) {
    // Being a purely cosmetic function,
    // we don't want cause more harm than good
    try {
        (e.target as Maybe<El>)?.select();
    } catch (e) {
        console.warn('selfSelect caused an error', e);
    }
}

export function resizeInput(e: HTMLInputElement, extraSpace?: number) {
    const size = e.value.length + (extraSpace ?? 0);
    e.style.width = `${size}ch`;
}

/**
 * Whrn elements have visually annoying focus ring,
 * it often covers other more important aspects
 * of the element likes it's active state styling.
 *
 * This function can then be applied as a click handler
 * with the effect of active element getting blurred
 * on mouse events, but not keyboard events.
 *
 * This is not to meddle with keyboard navigation
 * while still clearing up the visual smog
 * when interacting through pointer devices.
 */
export function blurActiveElement(event?: UIEvent | ReactUIEvent) {
    // Being a purely cosmetic function,
    // we don't want cause more harm than good
    try {
        // If used as an event handler, only react to mouse events
        if (event && 'type' in event && event.type !== 'click') return;

        (document.activeElement as Maybe<HTMLElement | SVGElement>)?.blur();
    } catch (e) {
        console.warn('blurActiveElement caused an error', e);
    }
}

export function noop() {}
export function pass<T>(v: T) {
    return v as T;
}
