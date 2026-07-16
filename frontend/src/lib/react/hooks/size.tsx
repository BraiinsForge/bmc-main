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

import { useState, useLayoutEffect, useMemo, type RefObject, useRef } from 'react';
import { throttle } from 'es-toolkit';

type Ref = RefObject<null | HTMLElement>;

interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}
function domRectToFloorRect(rect: DOMRect): Rect {
    return {
        x: Math.floor(rect.x),
        y: Math.floor(rect.y),
        width: Math.floor(rect.width),
        height: Math.floor(rect.height),
    };
}

export function useSize(target: Ref, throttleMs?: number): undefined | Rect {
    const [size, setSize] = useState<Rect>();
    const [element, setElement] = useState<HTMLElement | null>(null);

    const updateSize = useMemo(() => (throttleMs ? throttle(setSize, throttleMs) : setSize), [throttleMs]);

    // Track when the ref gets attached
    useLayoutEffect(() => {
        if (target.current !== element) setElement(target.current);
    });

    // Get initial size when element becomes available
    useLayoutEffect(() => {
        const domRect = element?.getBoundingClientRect();
        if (domRect) setSize(domRectToFloorRect(domRect));
        else setSize(undefined);
    }, [element]);

    // Observe resizes - use element state instead of ref so observer reattaches when element changes
    useLayoutEffect(() => {
        if (!element) return;

        const observer = new ResizeObserver(([entry]) => {
            if (entry) updateSize(domRectToFloorRect(entry.contentRect));
        });
        observer.observe(element);

        return () => observer.disconnect();
    }, [element, updateSize]);

    return size;
}

export function useSizeSelector<R>(target: Ref, selector: (size: undefined | Rect) => R): R {
    const size = useSize(target);
    return selector(size);
}

export function Sized<Element extends HTMLElement>(props: {
    render(ref: RefObject<null | Element>, size: undefined | Rect): ReactNode;
}) {
    const ref = useRef<null | Element>(null);
    const size = useSize(ref);
    return props.render(ref, size);
}
