import {
    Fragment,
    type Component,
    type HTMLAttributes,
    type SyntheticEvent,
    type UIEvent as ReactUIEvent,
    type FocusEvent as ReactFocusEvent,
} from 'react';
import type { JsonValue, JsonPrimitive } from 'type-fest';
import { isPlainObject } from 'es-toolkit';
import cn from 'clsx';

export { HelmetProvider, Helmet } from 'react-helmet-async';

export * from './hooks';
export * from './props';
export * from './render';

type MergeableObject = Record<string, undefined | JsonValue>;

export function query2state<Target extends MergeableObject, Source extends MergeableObject>(
    target: Target,
    source: Source,
    mapper?: (value: JsonValue, path: Array<string | number>) => NonNullable<JsonValue>,
): {
    target: Target;
    source: Source;
    found: Array<keyof Target>;
} {
    type K = keyof Target;

    const newTarget: Target = { ...target };
    const newSource: Source = { ...source };
    const found: K[] = [];

    Object.entries(newSource).forEach(([key, value]) => {
        if (!(key in newTarget)) return;
        let mapped: NonNullable<JsonValue>;

        // Object
        if (isPlainObject(value)) {
            const res: Record<string, JsonValue> = {};
            Object.entries(value as Record<string, JsonValue>).forEach(([k, v]) => {
                res[k] = mapper ? mapper(v as JsonPrimitive, [key, k]) : v;
            });

            mapped = res;
        }

        // Array
        else if (Array.isArray(value)) {
            mapped = value.map((d, i) => {
                return mapper ? mapper(d, [key, i]) : d;
            });
        }

        // Primitive
        else {
            mapped = mapper ? mapper(value as JsonPrimitive, [key]) : (value as NonNullable<JsonValue>);
        }

        newTarget[key as K] = mapped as Target[K];
        found.push(key as K);

        delete newSource[key];
    });

    return {
        target: newTarget,
        source: newSource,
        found,
    };
}

export function getToggleHandler<Prop extends string>(self: Component<unknown, { [k in Prop]: boolean }>, prop: Prop) {
    return (open?: boolean | unknown): void => {
        // @ts-expect-error: Funky biz with react state typing, but we can be pretty sure here…
        self.setState(s => ({ [prop]: typeof open === 'boolean' ? open : !s[prop] }));
    };
}

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

export interface CleanupQueue {
    add(fn: () => void): void;
    clear(): void;
}
export function getCleanupQueue(initialState?: Array<() => void>): CleanupQueue {
    const queue = new Set<() => void>(initialState);

    return {
        add(fn: () => void) {
            queue.add(fn);
        },
        clear() {
            queue.forEach(cleanFunction => {
                try {
                    cleanFunction();
                } catch (e) {
                    console.log('🧹 cleanupQueue error', e);
                }
            });
            queue.clear();
        },
    };
}

export interface ForcedChildStyling extends HTMLAttributes<HTMLDivElement> {
    id: string;
    childStyle: string;
}
export function ForcedChildStyling({ id, childStyle, ...rest }: ForcedChildStyling) {
    return (
        <Fragment>
            <style scoped children={`#${id} > * { ${childStyle} }`} />
            <div {...rest} id={id} className={cn(rest.className)} />
        </Fragment>
    );
}
