import { useState, useEffect } from 'react';
import * as pb from './proto';

export type AuthState = null | boolean;
export type Listener<R> = (store: Store) => R;
export type SubscribeResult = {
    unsubscribe(): void;
};

interface State {
    isAuthenticated: AuthState;
}

class Store {
    #state: Readonly<State> = {
        isAuthenticated: null,
    };
    #setState<Key extends keyof State>(key: Key, value: State[Key]): void {
        // @ts-expect-error: Only a type-guard
        // to prevent direct writes internally
        this.#state[key] = value;
        this.#notifyAllListeners();
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

    set isAuthenticated(value: boolean) {
        this.#setState('isAuthenticated', value);
    }
    get isAuthenticated(): AuthState {
        return this.#state.isAuthenticated;
    }

    login = async (password: string, signal: AbortSignal): Promise<void> => {
        await pb.rpc.auth.login({ password }, { signal });
        this.isAuthenticated = true;
    };
    logout = async (): Promise<void | pb.RpcStatus> => {
        try {
            await pb.rpc.auth.logout({});
        } catch {
            // Nothing to do here
        }

        this.isAuthenticated = false;
    };
    checkAuth = async (signal?: AbortSignal): Promise<boolean> => {
        try {
            const res = await pb.rpc.auth.isAuthenticated({}, { signal });
            this.isAuthenticated = res.value;
            return res.value;
        } catch ($) {
            this.isAuthenticated = false;
            const error = pb.parseError($);
            console.groupCollapsed(`%cAuth check failed (${error.rpc_reason_name})`, 'color: pink');
            console.log(error);
            console.groupEnd();
        }

        return this.isAuthenticated;
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
await store.checkAuth();

Object.assign(globalThis, { store });
if (process.env.NODE_ENV === 'development') console.log(store);
