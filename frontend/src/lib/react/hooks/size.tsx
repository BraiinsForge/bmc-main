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
