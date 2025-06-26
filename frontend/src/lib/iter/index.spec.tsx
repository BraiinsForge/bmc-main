import { describe, test, expect, rstest } from '@rstest/core';

import { deferred } from '../async';
import { handleAsyncIterable, abortableAsyncIterable } from './index';

describe('handleAsyncIterable', () => {
    test('throws', async () => {
        async function* gen() {
            yield 1;
            yield 2;
            throw new Error('Foo');
        }

        const tap = rstest.fn();
        const onEnd = rstest.fn();
        const onError = rstest.fn();
        const onAbort = rstest.fn();

        await handleAsyncIterable(gen(), { tap, onEnd, onError, onAbort });

        expect(tap).toHaveBeenCalledTimes(2);
        expect(onEnd).not.toHaveBeenCalled();
        expect(onError).toHaveBeenCalledTimes(1);
        expect(onAbort).not.toHaveBeenCalled();
    });
    test('aborted', async () => {
        async function* gen() {
            yield 1;
            yield 2;
            throw new DOMException('Aborted', 'AbortError');
        }

        const tap = rstest.fn();
        const onEnd = rstest.fn();
        const onError = rstest.fn();
        const onAbort = rstest.fn();

        await handleAsyncIterable(gen(), { tap, onEnd, onError, onAbort });

        expect(tap).toHaveBeenCalledTimes(2);
        expect(onEnd).not.toHaveBeenCalled();
        expect(onError).not.toHaveBeenCalled();
        expect(onAbort).toHaveBeenCalledTimes(1);
    });
    test('ends', async () => {
        async function* gen() {
            yield 1;
            yield 2;
            yield 3;
        }

        const tap = rstest.fn();
        const onEnd = rstest.fn();
        const onError = rstest.fn();
        const onAbort = rstest.fn();

        await handleAsyncIterable(gen(), { tap, onEnd, onError, onAbort });

        expect(tap).toHaveBeenCalledTimes(3);
        expect(onEnd).toHaveBeenCalledTimes(1);
        expect(onError).not.toHaveBeenCalled();
        expect(onAbort).not.toHaveBeenCalled();
    });
});

test('abortable AsyncIterable', async () => {
    const ctrl = new AbortController();
    const deff = deferred();
    const tout = window.setTimeout(() => deff.resolve('Hello'), 100);

    const gen = async function* () {
        yield 'Hello';
        await deff;
        yield 'World';
    };

    const items: string[] = [];
    for await (const item of abortableAsyncIterable(gen(), ctrl.signal)) items.push(item);

    expect(items).toEqual(['Hello', 'World']);
    window.clearTimeout(tout);
});
