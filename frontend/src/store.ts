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
import type { IntlShape } from 'react-intl';
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { boserChrome, loginOwned } from '@/lib/capabilities';
import { readBrand, type Brand } from '@/lib/brand';
import { intlFor, type Locale, SOURCE_LOCALE } from '@/lib/intl';
import { readSystem, type Capabilities } from '@/lib/system';

// The device saves a setup pick as one config write, and the page then loads its catalog.
// Either still unanswered after this is a stalled link,
// which would keep every switch read-only.
const SETUP_LANGUAGE_TIMEOUT_MS = 10_000;

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
    // Whether the device offers its languages.
    i18n: boolean;
    // Empty until `loadLanguages()` has listed them.
    languages: ReadonlyArray<pb.LanguageInfo>;
    // What the device took in the setup while the page
    // could not follow, until the two agree again.
    unspokenLanguage: null | string;
    // A setup pick is on its way, so no other
    // starts on this or the next setup step.
    isSavingLanguage: boolean;
    // English until the page's language is spoken, before the first render.
    locale: Locale;
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
        i18n: false,
        languages: [],
        unspokenLanguage: null,
        isSavingLanguage: false,
        locale: { tag: SOURCE_LOCALE },
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
    /** Unlike `state`, readable before `boot()`. */
    get locale(): Locale {
        return this.#state.locale;
    }
    /** Unlike `state`, readable before `boot()`. */
    get i18n(): boolean {
        return this.#state.i18n;
    }
    /** Unlike `state`, readable before `boot()`. */
    get languages(): ReadonlyArray<pb.LanguageInfo> {
        return this.#state.languages;
    }
    /** Unlike `state`, readable before `boot()`. */
    get unspokenLanguage(): null | string {
        return this.#state.unspokenLanguage;
    }
    /** Unlike `state`, readable before `boot()`. */
    get isSavingLanguage(): boolean {
        return this.#state.isSavingLanguage;
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
        void import('@/routes').then(({ default: router }) => router.navigate(URLS.auth.login));
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

    #speakAbort = pb.abort.get();
    /**
     * Speak `tag` here, its catalog loaded first, so whatever follows is already worded in it.
     * A catalog that does not load leaves the current language in place and returns `false`;
     * a later call wins over one whose catalog is still loading, which then returns `true`.
     */
    speak = async (tag: string, timeoutMs?: number): Promise<boolean> => {
        const { signal } = this.#speakAbort.replace();
        if (tag === this.#state.locale.tag) return true;
        let messages: Locale['messages'];
        if (this.#state.i18n) {
            try {
                ({ messages } = await pb.rpc.language.getWebCatalog({ language: tag }, { signal, timeoutMs }));
            } catch (error) {
                if (signal.aborted) return true;
                console.warn(`No web catalog for ${tag}; staying in ${this.#state.locale.tag}`, error);
                return false;
            }
            if (signal.aborted) return true;
        }
        this.#setState('locale', { tag, messages });
        return true;
    };

    /** The `IntlShape` of the current locale, for text formatted outside the React tree. */
    intl = (): IntlShape => intlFor(this.#state.locale);

    /**
     * Persist `tag` as the device language and speak it here once its catalog is loaded;
     * `false` when the device took it but the catalog did not load, so the page speaks it only after a reload.
     */
    setLanguage = async (tag: string, signal?: AbortSignal): Promise<boolean> => {
        await pb.rpc.config.setLanguage({ language: tag }, { signal });
        return this.speak(tag);
    };

    #languagesLoad: null | Promise<void> = null;
    /**
     * Lists the device's languages once, so each setup step does not carry the flags over the link again.
     * A failed list is retried on the next call.
     */
    loadLanguages = (): Promise<void> => {
        this.#languagesLoad ??= pb.rpc.language.listLanguages({}).then(
            ({ languages }) => this.#setState('languages', languages),
            error => {
                this.#languagesLoad = null;
                console.warn('No language list', error);
            },
        );
        return this.#languagesLoad;
    };

    /** `setLanguage` for the first setup, which has no session yet. */
    setSetupLanguage = async (tag: string): Promise<boolean> => {
        this.#setState('isSavingLanguage', true);
        try {
            await pb.rpc.init.setLanguage({ language: tag }, { timeoutMs: SETUP_LANGUAGE_TIMEOUT_MS });
            const spoken = await this.speak(tag, SETUP_LANGUAGE_TIMEOUT_MS);
            this.#setState('unspokenLanguage', spoken ? null : tag);
            return spoken;
        } finally {
            this.#setState('isSavingLanguage', false);
        }
    };

    setHardwareCapabilities = (caps: null | Capabilities): void => {
        this.#setState('hardwareCapabilities', caps);
    };
    setBrand = (brand: null | Brand): void => {
        this.#setState('brand', brand);
    };

    /**
     * Reads what the whole UI is keyed on, the language it speaks included;
     * throws when the device cannot be described.
     */
    boot = (): void => {
        const { capabilities: caps, i18n, language } = readSystem();
        this.#setState('i18n', i18n);
        // A page that boots speaks the device's own language.
        this.#setState('unspokenLanguage', null);
        if (language) this.#setState('locale', language);
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

Object.assign(globalThis, { store });
if (process.env.NODE_ENV === 'development') console.log(store);
