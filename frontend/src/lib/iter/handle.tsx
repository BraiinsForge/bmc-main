import { abort } from '../abort';

interface Opts<T> {
    tap(data: T): void;
    // The abort handler has to be explicitly defined or ignored!
    onAbort: null | Fn;
    onEnd?(): void;
    onError?(err: Error): void;
    onFinally?(): void;
}
export async function handleAsyncIterable<T>(stream: AsyncIterable<T>, opts: Opts<T>): Promise<void> {
    const { tap, onEnd, onError, onAbort, onFinally } = opts;

    try {
        for await (const message of stream) tap(message);
        onEnd?.();
        return;
    } catch (e) {
        // Abort error won't be reported to the generic
        // error handler if we have a special one
        if (onAbort && abort.is(e)) {
            onAbort();
            return;
        }

        onError?.(e as Error);

        // Without return here, the iteration will never stop
        return;
    } finally {
        onFinally?.();
    }
}
