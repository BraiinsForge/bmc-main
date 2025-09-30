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
