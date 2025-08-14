import { ConnectError, Code } from '@connectrpc/connect';
import type { Component } from 'react';

type Fn = () => void;
function getAbortControllerWithSignalHandlerAttached(onAbort?: Fn): AbortController {
    const ctrl = new AbortController();
    if (onAbort) ctrl.signal.addEventListener('abort', onAbort);

    return ctrl;
}

function assertAbortController(x: unknown): asserts x is AbortController {
    if (!x || typeof x !== 'object' || !('signal' in x)) {
        throw new Error('Aborter is not initialized');
    }
}

export class Aborter implements AbortController {
    #current?: AbortController;

    constructor(onAbort?: Fn) {
        this.#current = getAbortControllerWithSignalHandlerAttached(onAbort);
    }
    public replace(reason?: null | string, onAbort?: Fn): this {
        this.#current?.abort(reason);
        this.#current = getAbortControllerWithSignalHandlerAttached(onAbort);
        return this;
    }

    // Mimic AbortController interface
    public get signal(): AbortSignal {
        // If we were aborted, we'll just return a pre-aborted signal
        return this.#current?.signal ?? AbortSignal.abort();
    }
    public abort = (reason?: AbortSignal['reason']): void => {
        assertAbortController(this.#current);
        this.#current.abort(reason);
    };
    public destroy(reason?: AbortSignal['reason']): void {
        this.#current?.abort(reason);
        this.#current = undefined;
    }

    #length: number = 1;
    public get length(): number {
        return this.#length;
    }

    /**
     * Given an AbortSignal, the internal AbortController will be aborted when the abort signal is received.
     *
     * Given an AbortController, the attachment is made bi-directionally – that is:
     *  - the internal controller is aborted on outside signal
     *  - the external controller is aborted on internal signal
     *
     * The passed in object is duck-typed to support custom wrappers in addition to DOM built-ins.
     */
    public attach(other?: AbortController | AbortSignal): Aborter {
        if (!other) return this;

        // Abort self when other is aborted
        const signal = 'signal' in other ? other.signal : other;
        signal.addEventListener('abort', () => this.abort(signal.reason));

        // Abort other if
        //  - we are aborted
        //  - other is an AbortController
        if ('abort' in other && typeof other.abort === 'function') {
            this.signal.addEventListener('abort', () => other.abort(this.signal.reason));
        }

        this.#length++;
        return this;
    }
}

type Abortable = { abort: Fn };
type HasAbortables = Record<string, Abortable | Aborter | unknown>;

export const abort = {
    is(error: Maybe<Error & Rec> | unknown): boolean {
        const e = error as unknown;
        if (!e) return false;

        // variant returned by "fetch"
        if (e && (e as Rec).name === 'AbortError') return true;

        // variant returned from "grpcweb-transport"
        if (e instanceof ConnectError && e.code === Code.Canceled) return true;

        // Abort event object
        if (e instanceof Event && e.type === 'abort') return true;

        return false;
    },
    all(obj: HasAbortables | Component): void {
        Object.keys(obj).forEach(key => {
            const value = (obj as Rec)[key];

            // Skip if value is not something
            // remotely resembling an abort controller
            if (!value || typeof value !== 'object') return;

            // If this is a destroy operation,
            // we'll just call the destroy method
            if ('destroy' in value && typeof value.destroy === 'function') {
                value.destroy();
                return;
            }

            // …otherwise, we'll call the abort method
            // and delete the deref ourselves if requested
            if ('abort' in value && typeof value.abort === 'function') {
                value.abort();
                delete (obj as Rec)[key];
            }
        });
    },
    get(onAbort?: Fn): Aborter {
        return new Aborter(onAbort);
    },
    combine(...inputs: Array<AbortSignal | AbortController>): Aborter {
        const ctrl = new Aborter();
        inputs.forEach(x => {
            ctrl.attach(x);
        });
        return ctrl;
    },
};
