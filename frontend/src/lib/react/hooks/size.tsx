import { useState, useLayoutEffect, type RefObject } from 'react';
import useResizeObserver from '@react-hook/resize-observer';

type Ref = RefObject<null | HTMLElement>;

export function useSize(target: Ref): undefined | DOMRect {
    const [size, setSize] = useState<DOMRect>();

    useLayoutEffect(() => {
        setSize(target.current?.getBoundingClientRect());
    }, [target]);

    // Where the magic happens
    useResizeObserver(target, entry => setSize(entry.contentRect));

    return size;
}

export function useSizeSelector<R>(target: Ref, selector: (size: undefined | DOMRect) => R): R {
    const size = useSize(target);
    return selector(size);
}
