type DelayIface = Promise<void> & { cancel: () => void };
export function delay(timeoutMs: number): DelayIface {
    // These are needed for cancel method
    let id: number;
    let reject: undefined | Fn;

    // Create the base promise
    const p = new Promise<void>((resolve, $reject) => {
        reject = $reject;
        id = window.setTimeout(() => {
            resolve();
            reject = undefined; // Nullify the rejection reference
        }, timeoutMs);
    });

    return Object.assign(p, {
        cancel() {
            // Abort if the promise was already fullfiled
            if (!reject) return;

            // Otherwise cancel the interval and reject the promise
            clearInterval(id);
            reject();
        },
    });
}

/**
 * Creates a Promise with the `reject` and `resolve` functions placed as methods
 * on the promise object itself.
 *
 * @example
 * ```typescript
 * import { deferred } from "https://deno.land/std@$STD_VERSION/async/deferred.ts";
 *
 * const p = deferred<number>();
 * // ...
 * p.resolve(42);
 * ```
 */
export function deferred<T>(onResolve?: (value: T) => void, onReject?: (reason: any) => void): Deferred<T> {
    let methods: Pick<Deferred<T>, 'resolve' | 'reject'>;
    let state = 'pending';
    const promise = new Promise<T>((resolve, reject) => {
        methods = {
            async resolve(value: T | PromiseLike<T>) {
                const v = await value;
                onResolve?.(v);
                state = 'fulfilled';
                resolve(v);
            },
            reject(reason?: any) {
                onReject?.(reason);
                state = 'rejected';
                reject(reason);
            },
        };
    });

    Object.defineProperty(promise, 'state', { get: () => state });
    // @ts-expect-error: It's fine, trust me, I'm a doctor
    return Object.assign(promise, methods) as Deferred<T>;
}
export interface Deferred<T> extends Promise<T> {
    readonly state: 'pending' | 'fulfilled' | 'rejected';
    resolve(value?: T | PromiseLike<T>): void;
    reject(reason?: any): void;
}
