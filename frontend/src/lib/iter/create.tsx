import { delay } from '../async';

// An unambigous signal to skip an iteration
const SKIP = Symbol.for('SKIP_ITERATION');
type SkipSymbol = typeof SKIP;

type GetterContext = {
    delay: number;
    index: number;
    SKIP: SkipSymbol;
};
type Getter<T> = (ctx: GetterContext) => T | SkipSymbol;

export function createEndlessAsyncIterable<T>(conf: {
    delayMs: number;
    signal?: AbortSignal;
    get: Getter<T>;
}): AsyncGenerator<T> {
    async function* generator() {
        let i = -1;
        while (true) {
            if (conf.signal?.aborted) return;

            i++;
            await delay(conf.delayMs);

            const value = conf.get({ delay: conf.delayMs, index: i, SKIP });

            // Allow to skip iteration
            if (value !== SKIP) yield value;
        }
    }

    return generator();
}

export function createSingleMessageEndlessAsyncIterable<T>(get: Getter<T>, signal?: AbortSignal): AsyncGenerator<T> {
    let didRespond: boolean = false;
    return createEndlessAsyncIterable<T>({
        delayMs: 0,
        signal,
        get(ctx) {
            if (didRespond) return ctx.SKIP;
            didRespond = true;
            return get(ctx);
        },
    });
}
