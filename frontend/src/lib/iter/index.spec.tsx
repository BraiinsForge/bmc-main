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
