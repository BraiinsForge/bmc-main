import type { KeyboardEvent } from 'react';
import { Key } from 'ts-key-enum';

/**
 * IBM does some real fucking weird shit in their components.
 * One is that "Enter" key forces submit and if it's pressed
 * right after deleting the input value, it gets empty string
 * and sets the whole slider component as invalid.
 */
export function handleSliderParentKeyDownCapture(e: KeyboardEvent): void {
    if (e.target instanceof HTMLInputElement && e.key === Key.Enter) {
        e.preventDefault();
        e.stopPropagation();
    }
}
