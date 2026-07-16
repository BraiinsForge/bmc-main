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

import { deferred } from '../async';

/** The `reason` comes from `AbortSignal` thus must be `any` */
function createAbortError(reason?: any): DOMException {
    return new DOMException(reason ? `Aborted: ${reason}` : 'Aborted', 'AbortError');
}

/**
 * Adapted from Deno std 0.184.0.
 * Copyright 2018-2023 the Deno authors, licensed under MIT.
 *
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
