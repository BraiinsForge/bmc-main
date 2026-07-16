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
