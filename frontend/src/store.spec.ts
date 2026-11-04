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
import { cleanup, renderHook } from '@testing-library/react/pure';

import { store, useStore } from '@/store';
import * as pb from '@/proto';
import { languageInterceptor, mocks } from '@/proto/transport';
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

describe('store.boot', () => {
    afterEach(async () => {
        mocks.clear();
        await backToEnglish();
    });

    test('the language the page came with is spoken at once, catalog included', () => {
        const language = { tag: 'cs', messages: { save: 'Uložit' } };
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true, language } });

        store.boot();

        expect(store.locale).toEqual(language);
    });

    test('a page that came without a language stays in English', () => {
        Object.assign(window, { SYSTEM: deckSystemWire() });

        store.boot();

        expect(store.locale.tag).toBe('en');
    });
});

describe('the language an RPC asks for', () => {
    async function asked(): Promise<null | string> {
        const header = new Headers();
        await languageInterceptor(() => Promise.resolve({} as never))({ header } as never);
        return header.get('Accept-Language');
    }

    afterEach(async () => {
        mocks.clear();
        await backToEnglish();
    });

    test('is English before the page names one, not the browser’s own list', async () => {
        rstest.resetModules();
        const fresh = await import('@/proto/transport');
        const header = new Headers();

        await fresh.languageInterceptor(() => Promise.resolve({} as never))({ header } as never);

        expect(header.get('Accept-Language')).toBe('en');
    });

    test('is the one the page came with', async () => {
        const language = { tag: 'cs', messages: {} };
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true, language } });

        store.boot();

        expect(await asked()).toBe('cs');
    });

    test('goes out with every request the transport sends', async () => {
        const language = { tag: 'cs', messages: {} };
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true, language } });
        store.boot();
        let sent: null | string = null;
        const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = {
            getGeneralSettingsData: ({ header }) => {
                sent = header.get('Accept-Language');
                return pb.create(pb.GeneralSettingsDataResponseSchema);
            },
        };
        mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);

        await pb.rpc.config.getGeneralSettingsData({});

        expect(sent).toBe('cs');
    });

    test('follows the language the page goes on to speak', async () => {
        Object.assign(window, { SYSTEM: deckSystemWire() });
        store.boot();

        await store.speak('zh');

        expect(await asked()).toBe('zh');
    });

    test('stays put while a catalog fails to load', async () => {
        const language = { tag: 'cs', messages: {} };
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true, language } });
        store.boot();
        const methods: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            getWebCatalog: () => {
                throw new ConnectError('the device does not offer it', Code.InvalidArgument);
            },
        };
        mocks.service(pb.LanguageService, methods as ServiceMocks<typeof pb.LanguageService>);
        const warn = rstest.spyOn(console, 'warn').mockImplementation(() => {});

        await store.speak('zh');
        warn.mockRestore();

        expect(await asked()).toBe('cs');
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

describe('store.setSetupLanguage', () => {
    afterEach(async () => {
        mocks.clear();
        rstest.restoreAllMocks();
        rstest.useRealTimers();
        await backToEnglish();
    });

    test('a catalog that never arrives frees the switch and asks for a reload', async () => {
        rstest.useFakeTimers();
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        const init: Partial<ServiceMocks<typeof pb.services.InitialSetupService>> = { setLanguage: () => ({}) };
        mocks.service(pb.services.InitialSetupService, init as ServiceMocks<typeof pb.services.InitialSetupService>);
        const catalog: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            getWebCatalog: ({ conf }) => {
                conf({ delay: 600_000 });
                return { messages: {} };
            },
        };
        mocks.service(pb.LanguageService, catalog as ServiceMocks<typeof pb.LanguageService>);
        rstest.spyOn(console, 'warn').mockImplementation(() => {});

        const picked = store.setSetupLanguage('zh');
        await rstest.advanceTimersByTimeAsync(60_000);

        expect(store.isSavingLanguage, 'the switch is writable again').toBe(false);
        expect(store.unspokenLanguage).toBe('zh');
        await expect(picked).resolves.toBe(false);
    });

    test('a save the device never answers frees the switch', async () => {
        rstest.useFakeTimers();
        Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true } });
        store.boot();
        const init: Partial<ServiceMocks<typeof pb.services.InitialSetupService>> = {
            setLanguage: ({ conf }) => {
                conf({ delay: 600_000 });
                return {};
            },
        };
        mocks.service(pb.services.InitialSetupService, init as ServiceMocks<typeof pb.services.InitialSetupService>);

        const picked = store.setSetupLanguage('zh');
        const failed = expect(picked).rejects.toThrow();
        await rstest.advanceTimersByTimeAsync(60_000);

        expect(store.isSavingLanguage, 'the switch is writable again').toBe(false);
        await failed;
    });
});

describe('store.readGeneralSettings', () => {
    afterEach(() => {
        mocks.clear();
        rstest.restoreAllMocks();
        rstest.useRealTimers();
    });

    /** Answers the n-th read with `answers[n]`: its date after its delay, or a failure at once. */
    function answerReads(answers: Array<{ delay: number; date: pb.DateFormat } | 'fail'>): () => number {
        let reads = 0;
        const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = {
            getGeneralSettingsData: ({ conf }) => {
                const answer = answers[Math.min(reads, answers.length - 1)];
                reads += 1;
                if (answer === 'fail') throw new ConnectError('unreachable', Code.Unavailable);
                conf({ delay: answer.delay });
                return pb.create(pb.GeneralSettingsDataResponseSchema, { dateFormat: answer.date });
            },
        };
        mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);
        return () => reads;
    }

    test('a read overtaken by a later one resolves with the later answer', async () => {
        rstest.useFakeTimers();
        answerReads([
            { delay: 1_000, date: pb.DateFormat.M_D_YYYY_SLASH },
            { delay: 10, date: pb.DateFormat.YYYY_MM_DD_DASH },
        ]);

        const earlier = store.readGeneralSettings();
        const later = store.readGeneralSettings();
        const settled = Promise.all([earlier, later]);
        await rstest.advanceTimersByTimeAsync(2_000);

        const [first, second] = await settled;
        expect(first.dateFormat, 'the overtaken read is not left empty-handed').toBe(pb.DateFormat.YYYY_MM_DD_DASH);
        expect(second.dateFormat).toBe(pb.DateFormat.YYYY_MM_DD_DASH);
        expect(store.formats.date).toBe(pb.DateFormat.YYYY_MM_DD_DASH);
    });

    test('a read overtaken by a later one that fails rejects with its failure', async () => {
        rstest.useFakeTimers();
        answerReads([{ delay: 1_000, date: pb.DateFormat.M_D_YYYY_SLASH }, 'fail']);

        const earlier = store.readGeneralSettings();
        const later = store.readGeneralSettings();
        const settled = Promise.allSettled([earlier, later]);
        await rstest.advanceTimersByTimeAsync(2_000);

        const [first, second] = await settled;
        expect(first.status, 'the overtaken read does not resolve with nothing').toBe('rejected');
        expect(second.status).toBe('rejected');
        if (first.status === 'rejected') expect(pb.abort.is(first.reason), 'it carries the failure').toBe(false);
    });

    test('in a chain of three reads, every caller gets the newest answer', async () => {
        rstest.useFakeTimers();
        answerReads([
            { delay: 1_000, date: pb.DateFormat.M_D_YYYY_SLASH },
            { delay: 500, date: pb.DateFormat.DD_MM_YYYY_SLASH },
            { delay: 10, date: pb.DateFormat.YYYY_MM_DD_DASH },
        ]);

        const reads = [store.readGeneralSettings(), store.readGeneralSettings(), store.readGeneralSettings()];
        const settled = Promise.all(reads);
        await rstest.advanceTimersByTimeAsync(2_000);

        const answers = await settled;
        expect(answers.map(answer => answer.dateFormat)).toEqual([
            pb.DateFormat.YYYY_MM_DD_DASH,
            pb.DateFormat.YYYY_MM_DD_DASH,
            pb.DateFormat.YYYY_MM_DD_DASH,
        ]);
        expect(store.formats.date).toBe(pb.DateFormat.YYYY_MM_DD_DASH);
    });
});

describe('store.loadLanguages', () => {
    afterEach(() => {
        mocks.clear();
        rstest.restoreAllMocks();
    });

    test('lists the languages once, and again only after a list that failed', async () => {
        const languages = [pb.create(pb.LanguageInfoSchema, { tag: 'zh', nativeName: '简体中文' })];
        let requests = 0;
        const methods: Partial<ServiceMocks<typeof pb.LanguageService>> = {
            listLanguages: () => {
                requests += 1;
                if (requests === 1) throw new ConnectError('the link dropped', Code.Unavailable);
                return { languages };
            },
        };
        mocks.service(pb.LanguageService, methods as ServiceMocks<typeof pb.LanguageService>);
        rstest.spyOn(console, 'warn').mockImplementation(() => {});

        await store.loadLanguages();
        expect(store.languages).toEqual([]);

        await Promise.all([store.loadLanguages(), store.loadLanguages()]);
        await store.loadLanguages();

        expect(requests).toBe(2);
        expect(store.languages.map(x => x.tag)).toEqual(['zh']);
    });
});

describe('useStore', () => {
    afterEach(cleanup);

    test('a change between the first render and its subscription is not missed', () => {
        store.setHardwareCapabilities(deckCapabilities({ productName: 'before' }));
        let changed = false;

        const { result } = renderHook(() => {
            const productName = useStore(x => x.state.hardwareCapabilities.productName);
            if (!changed) {
                changed = true;
                store.setHardwareCapabilities(deckCapabilities({ productName: 'after' }));
            }
            return productName;
        });

        expect(result.current).toBe('after');
    });
});

describe('store.formats', () => {
    type GeneralSettingsMock = ServiceMocks<typeof pb.ConfigurationService>['getGeneralSettingsData'];

    function answerSettings(getGeneralSettingsData: GeneralSettingsMock): void {
        const methods: Partial<ServiceMocks<typeof pb.ConfigurationService>> = { getGeneralSettingsData };
        mocks.service(pb.ConfigurationService, methods as ServiceMocks<typeof pb.ConfigurationService>);
    }

    const settings = (
        dateFormat: pb.DateFormat,
        timeFormat: pb.TimeFormat,
        numberFormat: pb.NumberFormat,
        firstDayOfWeek: pb.Weekday,
    ) => pb.create(pb.GeneralSettingsDataResponseSchema, { dateFormat, timeFormat, numberFormat, firstDayOfWeek });

    const preferred = settings(
        pb.DateFormat.M_D_YYYY_SLASH,
        pb.TimeFormat.TIME_FORMAT_12_HOUR,
        pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL,
        pb.Weekday.SUNDAY,
    );
    const preferredFormats = {
        date: pb.DateFormat.M_D_YYYY_SLASH,
        time: pb.TimeFormat.TIME_FORMAT_12_HOUR,
        number: pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL,
        firstDay: pb.Weekday.SUNDAY,
    };

    afterEach(() => {
        mocks.clear();
        rstest.restoreAllMocks();
    });

    test("are read from the device's settings", async () => {
        answerSettings(() => preferred);

        await store.loadFormats();

        expect(store.formats).toEqual(preferredFormats);
    });

    test("an unspecified one keeps the device's default", async () => {
        answerSettings(() =>
            settings(
                pb.DateFormat.UNSPECIFIED,
                pb.TimeFormat.TIME_FORMAT_UNSPECIFIED,
                pb.NumberFormat.UNSPECIFIED,
                pb.Weekday.UNSPECIFIED,
            ),
        );

        await store.loadFormats();

        expect(store.formats).toEqual({
            date: pb.DateFormat.DD_MM_YYYY_DOT,
            time: pb.TimeFormat.TIME_FORMAT_24_HOUR,
            number: pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL,
            firstDay: pb.Weekday.MONDAY,
        });
    });

    test('a load that fails leaves them as they were', async () => {
        answerSettings(() => preferred);
        await store.loadFormats();
        answerSettings(() => {
            throw new ConnectError('the link dropped', Code.Unavailable);
        });
        rstest.spyOn(console, 'warn').mockImplementation(() => {});

        await store.loadFormats();

        expect(store.formats).toEqual(preferredFormats);
    });

    test('a later load wins over one still loading', async () => {
        answerSettings(({ conf }) => {
            conf({ delay: 20 });
            return settings(
                pb.DateFormat.YYYY_MM_DD_DOT,
                pb.TimeFormat.TIME_FORMAT_24_HOUR,
                pb.NumberFormat.DOT_GROUP_COMMA_DECIMAL,
                pb.Weekday.WEDNESDAY,
            );
        });
        const earlier = store.loadFormats();
        answerSettings(() => preferred);

        await store.loadFormats();
        await earlier;

        expect(store.formats).toEqual(preferredFormats);
    });

    function answerSession(isAuthenticated: boolean): void {
        const auth: Partial<ServiceMocks<typeof pb.AuthenticationService>> = {
            isAuthenticated: () => ({ value: isAuthenticated }),
        };
        const system: Partial<ServiceMocks<typeof pb.SystemService>> = {
            hasPassword: () => ({ value: true }),
        };
        mocks.service(pb.AuthenticationService, auth as ServiceMocks<typeof pb.AuthenticationService>);
        mocks.service(pb.SystemService, system as ServiceMocks<typeof pb.SystemService>);
    }

    test.each([
        ['a session check that finds a session reads them', true, 1],
        ['a session check that finds none reads nothing', false, 0],
    ])('%s', async (_case, isAuthenticated, reads) => {
        answerSession(isAuthenticated);
        const load = rstest.spyOn(store, 'loadFormats').mockResolvedValue();

        await store.fetchSessionInfo();

        expect(load).toHaveBeenCalledTimes(reads);
    });

    test('a session checked again reads them again, as at a login after the session was lost', async () => {
        answerSession(true);
        const load = rstest.spyOn(store, 'loadFormats').mockResolvedValue();
        await store.fetchSessionInfo();

        await store.fetchSessionInfo();

        expect(load).toHaveBeenCalledTimes(2);
    });

    test('a found session is published with them already read', async () => {
        store.setHardwareCapabilities(deckCapabilities());
        answerSession(true);
        const before = store.state.sessionInfo;
        const expected = {
            date: pb.DateFormat.YYYY_M_D_SLASH,
            time: pb.TimeFormat.TIME_FORMAT_24_HOUR,
            number: pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL,
            firstDay: pb.Weekday.WEDNESDAY,
        };
        answerSettings(({ conf }) => {
            conf({ delay: 20 });
            return settings(expected.date, expected.time, expected.number, expected.firstDay);
        });
        let atPublish: unknown = null;
        const { unsubscribe } = store.subscribe(s => {
            if (s.state.sessionInfo !== before) atPublish ??= s.formats;
        });

        await store.fetchSessionInfo();
        unsubscribe();

        expect(atPublish).toEqual(expected);
    });

    test('a later session check wins over one still reading them', async () => {
        store.setHardwareCapabilities(deckCapabilities());
        answerSession(true);
        answerSettings(({ conf }) => {
            conf({ delay: 20 });
            return preferred;
        });
        const earlier = store.fetchSessionInfo();
        // Past the earlier check's session answers, to where it waits on the preferences.
        await new Promise(resolve => setTimeout(resolve, 5));
        answerSession(false);

        await store.fetchSessionInfo();
        await earlier;

        expect(store.state.sessionInfo.isAuthenticated).toBe(false);
    });
});
