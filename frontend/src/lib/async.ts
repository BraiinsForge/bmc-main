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
