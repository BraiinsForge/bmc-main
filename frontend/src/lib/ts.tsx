/**
 * Used for exhaustive switch checking
 * @see https://stackoverflow.com/questions/39419170/how-do-i-check-that-a-switch-block-is-exhaustive-in-typescript
 */
export function assertUnreachable(x: never, label?: string): never {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    throw new Error(`${message} - ${x}`);
}
