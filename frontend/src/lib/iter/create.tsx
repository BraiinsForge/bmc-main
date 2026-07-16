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
