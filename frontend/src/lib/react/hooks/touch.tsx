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
