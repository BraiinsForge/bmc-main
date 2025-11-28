import { useSyncExternalStore } from 'react';

/**
 * Detects if the device has a coarse pointer (touch screen).
 * Uses matchMedia to detect `(pointer: coarse)` which indicates
 * the primary input is a touch screen rather than a mouse.
 *
 * This is more reliable than checking for touch events because:
 * - Laptops with touchscreens still have `pointer: fine` as primary
 * - It correctly identifies phones/tablets as touch-primary devices
 */

const query = '(pointer: coarse)';

function getSnapshot(): boolean {
    return window.matchMedia(query).matches;
}

function getServerSnapshot(): boolean {
    return false;
}

function subscribe(callback: () => void): () => void {
    const mediaQuery = window.matchMedia(query);
    mediaQuery.addEventListener('change', callback);
    return () => mediaQuery.removeEventListener('change', callback);
}

export function useIsTouchDevice(): boolean {
    return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
