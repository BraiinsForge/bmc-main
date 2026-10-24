// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';

import { ConnectError, Code } from '@connectrpc/connect';

import { store } from '@/store';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { stubLocation } from '@/mocks/location';
import { deckCapabilities, deckSystemWire } from '@/pages/workspace/Display/capabilities.fixture';

// The real routes pull in every page; the store only needs their navigate.
rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

afterEach(() => {
    store.setHardwareCapabilities(null);
});

/** Back in English without a request: a device without its languages loads no catalog. */
async function backToEnglish(): Promise<void> {
    Object.assign(window, { SYSTEM: deckSystemWire() });
    store.boot();
    await store.speak('en');
    Reflect.deleteProperty(window, 'SYSTEM');
}

describe('store.state', () => {
    test('throws before boot() loaded the capabilities', () => {
        expect(() => store.state).toThrow('BUG');
    });

    test('is the same object until something changes', () => {
        store.setHardwareCapabilities(deckCapabilities());

        expect(store.state).toBe(store.state);
    });

    test('is a new object after a change', () => {
        store.setHardwareCapabilities(deckCapabilities());
        const before = store.state;

        store.setHardwareCapabilities(deckCapabilities());

        expect(store.state).not.toBe(before);
    });

    test('is frozen', () => {
        store.setHardwareCapabilities(deckCapabilities());

        expect(Object.isFrozen(store.state)).toBe(true);
    });
});

describe('store.goToLogin', () => {
    test("a boser-managed device leaves for boser's login", () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: true }));
        const assign = rstest.fn();
        stubLocation({ assign });

        store.goToLogin();

        expect(assign).toHaveBeenCalledWith('/bos/login');
    });

    test('a standalone Deck goes to its own login', async () => {
        store.setHardwareCapabilities(deckCapabilities({ boserManaged: false }));
        const { default: router } = await import('@/routes');

        store.goToLogin();

        await rstest.waitFor(() => expect(router.navigate).toHaveBeenCalledWith('/login'));
    });
});

describe('store.speak', () => {
    let catalogReads = 0;

    type CatalogMock = ServiceMocks<typeof pb.LanguageService>['getWebCatalog'];

    /** The web catalog answering with `answer`, each read counted. */
    function answerCatalog(answer: CatalogMock): void {
        const methods: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            getWebCatalog: input => {
                catalogReads += 1;
                return answer(input);
            },
        };
        mocks.service(pb.LanguageService, methods as ServiceMocks<typeof pb.LanguageService>);
    }

    /** Boot as a device that offers its languages, its web catalog answering with `answer`. */
    function bootWithLanguages(answer: CatalogMock): void {
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        answerCatalog(answer);
    }

    afterEach(async () => {
        mocks.clear();
        rstest.restoreAllMocks();
        catalogReads = 0;
        await backToEnglish();
    });

    test('a language is spoken once its catalog has loaded', async () => {
        bootWithLanguages(() => ({ messages: { save: 'Uložit' } }));

        await store.speak('cs');

        expect(store.locale).toEqual({ tag: 'cs', messages: { save: 'Uložit' } });
    });

    test("an English pack's catalog is loaded like any other", async () => {
        bootWithLanguages(() => ({ messages: { colour: 'Colour' } }));

        await store.speak('en-AU');

        expect(catalogReads).toBe(1);
        expect(store.locale).toEqual({ tag: 'en-AU', messages: { colour: 'Colour' } });
    });

    test('a catalog that does not load leaves the language in place', async () => {
        bootWithLanguages(() => {
            throw new ConnectError('the device does not offer it', Code.InvalidArgument);
        });
        rstest.spyOn(console, 'warn').mockImplementation(() => {});

        await store.speak('cs');

        expect(store.locale.tag).toBe('en');
    });

    test('a device without its languages loads no catalog', async () => {
        Object.assign(window, { SYSTEM: deckSystemWire() });
        store.boot();
        answerCatalog(() => ({ messages: { save: 'Uložit' } }));

        await store.speak('cs');

        expect(catalogReads).toBe(0);
        expect(store.locale).toEqual({ tag: 'cs' });
    });

    test('a later language wins over one whose catalog is still loading', async () => {
        bootWithLanguages(({ req, conf }) => {
            conf({ delay: 20 });
            const messages: Record<string, string> = req.language === 'cs' ? { save: 'Uložit' } : {};
            return { messages };
        });
        const warn = rstest.spyOn(console, 'warn').mockImplementation(() => {});

        const czech = store.speak('cs');
        await store.speak('en-GB');
        await czech;

        expect(store.locale).toEqual({ tag: 'en-GB', messages: {} });
        expect(warn).not.toHaveBeenCalled();
    });
});

describe('store.speakBootLanguage', () => {
    afterEach(async () => {
        mocks.clear();
        await backToEnglish();
        Reflect.deleteProperty(window, 'LANGUAGE');
    });

    test('the language the page came with is spoken at once, catalog included', () => {
        Object.assign(window, { LANGUAGE: { tag: 'cs', messages: { save: 'Uložit' } } });

        store.speakBootLanguage();

        expect(store.locale).toEqual({ tag: 'cs', messages: { save: 'Uložit' } });
    });

    test('a page that came without a language stays in English', () => {
        Object.assign(window, { LANGUAGE: null });

        store.speakBootLanguage();

        expect(store.locale.tag).toBe('en');
    });
});

describe('store.setLanguage', () => {
    afterEach(async () => {
        mocks.clear();
        rstest.restoreAllMocks();
        await backToEnglish();
    });

    test('the device takes the language before the page speaks it', async () => {
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        const taken: string[] = [];
        const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = {
            setLanguage: ({ req }) => {
                taken.push(req.language);
                expect(store.locale.tag).toBe('en');
                return {};
            },
        };
        mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);
        const catalog: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            getWebCatalog: () => ({ messages: {} }),
        };
        mocks.service(pb.LanguageService, catalog as ServiceMocks<typeof pb.LanguageService>);

        await store.setLanguage('en-US');

        expect(taken).toEqual(['en-US']);
        expect(store.locale.tag).toBe('en-US');
    });

    test('a language the device took but whose catalog did not load is reported, not spoken', async () => {
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        const config: Partial<ServiceMocks<typeof pb.ConfigurationService>> = { setLanguage: () => ({}) };
        mocks.service(pb.ConfigurationService, config as ServiceMocks<typeof pb.ConfigurationService>);
        const catalog: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            getWebCatalog: () => {
                throw new ConnectError('unreachable', Code.Unavailable);
            },
        };
        mocks.service(pb.LanguageService, catalog as ServiceMocks<typeof pb.LanguageService>);
        rstest.spyOn(console, 'warn').mockImplementation(() => {});

        await expect(store.setLanguage('cs')).resolves.toBe(false);

        expect(store.locale.tag).toBe('en');
    });

    test('a language the device refuses is not spoken', async () => {
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = {
            setLanguage: () => {
                throw new ConnectError('not offered', Code.InvalidArgument);
            },
        };
        mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);

        await expect(store.setLanguage('en-US')).rejects.toThrow('not offered');

        expect(store.locale.tag).toBe('en');
    });
});
