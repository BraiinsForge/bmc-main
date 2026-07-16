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

import { useState, useEffect } from 'react';
import * as pb from '@/proto';

export type Listener<R> = (store: Store) => R;
export type SubscribeResult = {
    unsubscribe(): void;
};

interface SessionInfo {
    isAuthenticated: null | boolean;
    hasPassword: null | boolean;
}
interface State {
    sessionInfo: SessionInfo;
}

class Store {
    #state: Readonly<State> = {
        sessionInfo: {
            isAuthenticated: null,
            hasPassword: null,
        },
    };
    #setState<Key extends keyof State>(key: Key, value: State[Key] | ((currentState: State[Key]) => State[Key])): void {
        // @ts-expect-error: Only a type-guard
        // to prevent direct writes internally
        this.#state[key] = typeof value === 'function' ? value(this.#state[key]) : value;
        this.#notifyAllListeners();
    }
    get state(): Readonly<State> {
        return Object.freeze({ ...this.#state });
    }

    #listeners = new Set<Listener<any>>();
    subscribe<R>(listener: Listener<R>): SubscribeResult {
        this.#listeners.add(listener);
        return {
            unsubscribe: () => this.#listeners.delete(listener),
        };
    }
    #notifyAllListeners(): void {
        for (const listener of this.#listeners) listener(this);
    }

    login = async (password: string, signal: AbortSignal): Promise<void> => {
        await pb.rpc.auth.login({ password }, { signal });
        await this.fetchSessionInfo();
    };
    logout = async (): Promise<void | pb.RpcStatus> => {
        try {
            await pb.rpc.auth.logout({});
        } catch {
            // Nothing to do here
        }
        this.fetchSessionInfo();
    };

    #fetchSessionInfoAbort = pb.abort.get();
    fetchSessionInfo = async (): Promise<void> => {
        const { signal } = this.#fetchSessionInfoAbort.replace();
        const res: SessionInfo = { isAuthenticated: false, hasPassword: null };

        try {
            const x = await pb.rpc.auth.isAuthenticated({}, { signal });
            res.isAuthenticated = x.value;
        } catch ($) {
            res.isAuthenticated = false;
            const error = pb.parseError($);
            console.groupCollapsed(`%cAuth check failed (${error.rpc_reason_name})`, 'color: pink');
            console.log(error);
            console.groupEnd();
        }

        if (res.isAuthenticated) {
            try {
                const x = await pb.rpc.sys.hasPassword({}, { signal });
                res.hasPassword = x.value;
            } catch ($) {
                if (pb.abort.is($)) return;

                const error = pb.parseError($);
                console.groupCollapsed(
                    `%cFailed to check if user has password (${error.rpc_reason_name})`,
                    'color: pink',
                );
                console.log(error);
                console.groupEnd();
            }
        }

        this.#setState('sessionInfo', res);
    };
}

export const store = new Store();

export function useStore<Res>(getter: (store: Store) => Res) {
    const [state, setState] = useState<Res>(getter(store));

    useEffect(() => {
        const x = store.subscribe(s => setState(getter(s)));
        return x.unsubscribe;
    });

    return state;
}

// Fetching the user info as soon as possible
// makes us aware of their auth status
await store.fetchSessionInfo();

Object.assign(globalThis, { store });
if (process.env.NODE_ENV === 'development') console.log(store);
