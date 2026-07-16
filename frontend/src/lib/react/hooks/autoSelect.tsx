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

import { useRef, useCallback, useState, createElement, type Ref, type MouseEventHandler } from 'react';
import copy2clipboard from 'copy-to-clipboard';
import { selectNodeContent } from '@/lib/dom';
import { mergeRefs, type ElementKind, type PropsWithKind, type ElementTypeFromKind } from '../props';

interface UseAutoSelectConfig {
    disabled?: boolean;
    autoCopy?: boolean;
}
type UseAutoSelectReturn<RefType> = [
    ref: Ref<RefType>,
    propsToPass: {
        onClick: MouseEventHandler<RefType>;
        tabIndex?: number;
    },
];

/**
 * Helper hook to select the text of an element when it is clicked.
 *
 * @example
 * const [ref, onClick] = useAutoSelect<HTMLSpanElement>();
 * return <span ref={ref} onClick={onClick} children="…" />;
 */
export function useAutoSelect<T extends HTMLElement>(conf?: UseAutoSelectConfig): UseAutoSelectReturn<T> {
    const ref = useRef<T>(null);
    const autoCopy = conf?.autoCopy ?? false;
    const disabled = conf?.disabled ?? false;

    const select = useCallback(() => {
        if (disabled) return;

        const element = ref.current;
        if (!element) return;

        if (autoCopy) {
            const text = element.textContent ?? '';
            copy2clipboard(text);
        }
        selectNodeContent(element);
    }, [autoCopy, disabled]);

    const $props: UseAutoSelectReturn<T>[1] = { onClick: select };
    return [ref, $props];
}

export interface AutoSelectProps<T extends HTMLElement> extends UseAutoSelectConfig {
    render(...props: UseAutoSelectReturn<T>): ReactElement;
}

/**
 * This component is used to wrap text that should be selectable by clicking on it.
 * @example <AutoSelect render={(ref, fn) => <span ref={ref} onClick={fn} children="…" />} />
 */
export function AutoSelect<T extends HTMLElement>({ render, ...rest }: AutoSelectProps<any>) {
    return render(...useAutoSelect<T>(rest));
}

export type AutoSelectedProps<T extends ElementKind> = PropsWithKind<T> &
    UseAutoSelectConfig & { innerRef?: Ref<ElementTypeFromKind<T>> };

/**
 * Version of `AutoComplete` that accepts a `kind` prop to specify the element type
 * and handles the passage of extra props to that element.
 *
 * The extra props are typed according to the `kind` prop as well.
 */
export function AutoSelected<T extends ElementKind = 'span'>(
    props: AutoSelectedProps<T>,
): ReactElement<ElementTypeFromKind<T>> {
    const {
        // For the AutoSelect
        kind = 'span',
        innerRef,
        autoCopy,
        disabled,
        // DOM attributes
        style,
        ...rest
    } = props;
    return (
        <AutoSelect
            disabled={disabled}
            autoCopy={autoCopy}
            render={(ref, autoSelectionProps) => {
                const $props = {
                    ...rest,
                    ...autoSelectionProps,
                    style: {
                        cursor: autoCopy ? 'copy' : 'cell',
                        ...style,
                    } satisfies CSSProperties,
                    ref: mergeRefs(ref, innerRef),
                };
                return createElement(kind, $props);
            }}
        />
    );
}

/**
 * This hook is used to wait for a promise to resolve and then return the value.
 * @example const value = useAwaited(promise);
 */
export function useAwaited<T>(promise: Promise<T>, onError?: (e: Error) => void): T | null {
    const [value, setValue] = useState<T | null>(null);

    promise.then(setValue, e => {
        onError?.(e);
        setValue(null);
    });

    return value;
}

/** This component is used to wait for a promise to resolve and then render the value */
export function Awaited<T extends ReactNode>(props: { promise: Promise<T> }): ReactNode {
    return useAwaited<ReactNode>(props.promise);
}
