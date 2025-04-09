import { deferred } from '../async';

/** The `reason` comes from `AbortSignal` thus must be `any` */
function createAbortError(reason?: any): DOMException {
    return new DOMException(reason ? `Aborted: ${reason}` : 'Aborted', 'AbortError');
}

/**
 * Make AsyncIterable abortable with the given signal.
 *
 * @example
 * ```typescript
 * const p = async function* () {
 *   yield "Hello";
 *   await delay(1000);
 *   yield "World";
 * };
 *
 * const c = new AbortController();
 * setTimeout(c.abort, 100);
 *
 * // Throws `DOMException` after 100 ms
 * // and items become `["Hello"]`
 * const items: string[] = [];
 * for await (const item of abortableAsyncIterable(p(), c.signal)) items.push(item);
 * ```
 *
 * @see https://github.com/denoland/deno_std/blob/0.184.0/async/abortable.ts
 */
export async function* abortableAsyncIterable<T>(
    iterable: AsyncIterable<T>,
    signal: AbortSignal,
    onMessage?: (message: T) => void,
): AsyncGenerator<T> {
    if (signal.aborted) throw createAbortError(signal.reason);

    const waiter = deferred<never>();
    const abort = () => waiter.reject(createAbortError(signal.reason));
    signal.addEventListener('abort', abort, { once: true });

    const it = iterable[Symbol.asyncIterator]();
    while (true) {
        const { done, value } = await Promise.race([waiter, it.next()]);
        if (done) {
            signal.removeEventListener('abort', abort);
            return;
        }
        onMessage?.(value);
        yield value;
    }
}
