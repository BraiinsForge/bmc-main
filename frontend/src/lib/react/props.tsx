import type { Ref, RefCallback, JSX, DetailedHTMLProps, HTMLAttributes } from 'react';

/**
 * Copyright IBM Corp. 2016, 2018
 *
 * This source code is licensed under the Apache-2.0 license found in the
 * LICENSE file in the root directory of this source tree.
 */

export function mergeRefs<T>(...refs: Array<Maybe<Ref<T>>>): RefCallback<T> {
    return (el: T) => {
        refs.forEach(ref => {
            if (typeof ref === 'function') ref(el);
            // @ts-expect-error: https://github.com/facebook/react/issues/13029#issuecomment-410002316
            else if (Object(ref) === ref) ref.current = el;
        });
    };
}

export function populateRef<T>(ref: Maybe<Ref<T>>, value: T): void {
    if (!ref) return;

    if (typeof ref === 'function') ref(value);
    else if (Object(ref) === ref) ref.current = value;
}

export type ElementKind = keyof JSX.IntrinsicElements;

type InferElementFromDetailedProps<T extends DetailedHTMLProps<HTMLAttributes<unknown>, unknown>> =
    T extends DetailedHTMLProps<HTMLAttributes<infer ElementType>, unknown> ? ElementType : never;
export type ElementTypeFromKind<T extends ElementKind> = InferElementFromDetailedProps<JSX.IntrinsicElements[T]>;

export type PropsWithKind<T extends ElementKind> = JSX.IntrinsicElements[T] & {
    kind?: T;
};
