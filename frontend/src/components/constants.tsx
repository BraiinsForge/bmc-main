import type { MouseEvent, KeyboardEvent, SyntheticEvent, HTMLAttributes } from 'react';

function handleSpaceEnter(event: Maybe<KeyboardEvent>, callback: (e: KeyboardEvent) => void) {
    if (!event?.key) return;
    if (event.key === 'Enter' || event.key === ' ') callback(event);
}

const checkbox = (checked: boolean | 'mixed', handler: AnyFunction) => ({
    tabIndex: 0,
    role: 'checkbox',
    'aria-checked': typeof checked === 'boolean' ? checked : checked,
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => handleSpaceEnter(event, handler),
    onClick: handler,
});

const button = (handler: (e: SyntheticEvent) => void, blurOnClick?: boolean) => ({
    tabIndex: 0,
    role: 'button',
    onKeyDown: (event: KeyboardEvent) => handleSpaceEnter(event, handler),
    onClick: (event: MouseEvent) => {
        handler(event);

        // Blur the `activeElement` not to leave the clicked element in focused state
        // (the focus only really makes sense for keyboard navigation)
        // @ts-expect-error: Missing blur method in DOM api
        if (blurOnClick) document.activeElement?.blur();
    },
});

type ArrowHandler = (direction: -1 | 1) => void;
type ToggleHandler = (e: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>) => void;
const listItem = (
    onToggle?: Maybe<ToggleHandler>,
    onArrow?: Maybe<ArrowHandler>,
    labelledby?: string,
    describedby?: string,
) => {
    const r: HTMLAttributes<HTMLElement> = {
        tabIndex: -1,
        role: 'listItem',
        'aria-labelledby': labelledby || undefined,
        'aria-describedby': describedby || undefined,
    };

    if (onToggle || onArrow) {
        r.tabIndex = 0;
        r.onKeyDown = (e: KeyboardEvent<HTMLElement>): void => {
            if (onToggle && (e.key === ' ' || e.key === 'Enter')) {
                e.preventDefault();
                e.stopPropagation();
                onToggle(e);
            }
            if (onArrow && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
                e.preventDefault();
                e.stopPropagation();
                onArrow(e.key === 'ArrowUp' ? -1 : 1);
            }
        };
    }
    if (onToggle) {
        r.onClick = (e: MouseEvent<HTMLElement>): void => {
            onToggle(e);
            e.stopPropagation();
            e.preventDefault();
        };
    }

    return r;
};

export const ARIA = { checkbox, listItem, button };
