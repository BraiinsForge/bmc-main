import { useState, useLayoutEffect, type RefObject, useRef } from 'react';
import useResizeObserver from '@react-hook/resize-observer';

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

export function useSize(target: Ref): undefined | Rect {
    const [size, setSize] = useState<Rect>();

    const element = target.current;
    useLayoutEffect(() => {
        const domRect = element?.getBoundingClientRect();
        // Floor the values to avoid needless floating updates
        if (domRect) setSize(domRectToFloorRect(domRect));
        else setSize(undefined);
    }, [element]);

    // Where the magic happens
    useResizeObserver(target, entry => setSize(domRectToFloorRect(entry.contentRect)));

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
