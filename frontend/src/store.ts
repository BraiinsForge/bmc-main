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
import { URLS } from '@/constants';
import { boserChrome, loginOwned } from '@/lib/capabilities';
import { readBrand, type Brand } from '@/lib/brand';
import { readSystem, type Capabilities } from '@/lib/system';

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
    hardwareCapabilities: Capabilities;
    brand: null | Brand;
}
// Capabilities exist only after `boot()`; the public `state` hides that gap.
type StoreState = Omit<State, 'hardwareCapabilities'> & { hardwareCapabilities: null | State['hardwareCapabilities'] };

class Store {
    #state: Readonly<StoreState> = Object.freeze({
        sessionInfo: {
            isAuthenticated: null,
            hasPassword: null,
        },
        hardwareCapabilities: null,
        brand: null,
    });
    #setState<Key extends keyof StoreState>(
        key: Key,
        value: StoreState[Key] | ((currentState: StoreState[Key]) => StoreState[Key]),
    ): void {
        const next = typeof value === 'function' ? value(this.#state[key]) : value;
        this.#state = Object.freeze({ ...this.#state, [key]: next });
        this.#notifyAllListeners();
    }
    get state(): Readonly<State> {
        if (this.#state.hardwareCapabilities === null) {
            throw new Error('BUG: store read before boot() loaded the capabilities');
        }
        return this.#state as Readonly<State>;
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
    logout = async (): Promise<void> => {
        try {
            await pb.rpc.auth.logout({});
        } catch {
            // The login page is where we go either way
        }
        await this.fetchSessionInfo();
        // Root leaves once the session ends; without a password it never does, so leave from here.
        if (this.state.sessionInfo.isAuthenticated) this.goToLogin();
    };
    /** The login is boser's on a device it manages; ours would only let the user past our own session. */
    goToLogin = (): void => {
        if (loginOwned(this.state.hardwareCapabilities)) {
            window.location.assign(URLS.boser.login);
            return;
        }
        // Lazy: the routes import Root, which imports the store.
        import('@/routes').then(({ default: router }) => router.navigate(URLS.auth.login));
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

    setHardwareCapabilities = (caps: null | Capabilities): void => {
        this.#setState('hardwareCapabilities', caps);
    };
    setBrand = (brand: null | Brand): void => {
        this.#setState('brand', brand);
    };

    /** Reads what the whole UI is keyed on; throws when the device cannot be described. */
    boot = (): void => {
        const { capabilities: caps } = readSystem();
        this.setHardwareCapabilities(caps);
        // system.js carries the brand script only where boser manages the device.
        this.setBrand(boserChrome(caps) ? readBrand() : null);
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
