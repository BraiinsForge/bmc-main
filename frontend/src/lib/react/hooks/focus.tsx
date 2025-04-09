import { useEffect, useRef, type RefObject } from 'react';

export function useFocus<T extends HTMLElement>(): RefObject<null | T> {
    const focusElement = useRef<T>(null);
    const { current: element } = focusElement;

    useEffect(() => {
        element?.focus();
    }, [element]);

    return focusElement;
}
