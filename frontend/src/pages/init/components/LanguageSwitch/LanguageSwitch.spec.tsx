// Copyright (C) 2026  Braiins Systems s.r.o.
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
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { Code, ConnectError } from '@connectrpc/connect';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { store } from '@/store';
import { deckSystemWire } from '@/pages/workspace/Display/capabilities.fixture';

import { LanguageSwitch } from './LanguageSwitch';

// The real routes import every page, and the store needs only `navigate`.
rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

// Flags, so the closed switch shows the flag alone as it does on a device.
const flag = pb.create(pb.IconSchema, { mimeType: 'image/svg+xml', data: 'PHN2Zy8+' });
const languages = [
    pb.create(pb.LanguageInfoSchema, { tag: 'en-GB', nativeName: 'English (UK)', flag }),
    pb.create(pb.LanguageInfoSchema, { tag: 'zh', nativeName: '简体中文', flag }),
];

/** `null` boots as a device whose language is no longer listed, which sends none. */
function boot(i18n: boolean, tag: null | string = 'en-GB'): void {
    const language = i18n && tag ? { tag } : undefined;
    Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n, language } });
    store.boot();
    Reflect.deleteProperty(window, 'SYSTEM');
}

function renderSwitch(): ReturnType<typeof rstest.fn> {
    const onErrors = rstest.fn();
    render(
        <IntlProvider locale="en">
            <LanguageSwitch onErrors={onErrors} />
        </IntlProvider>,
    );
    return onErrors;
}

afterEach(async () => {
    cleanup();
    // A save still on its way would leave the next test's switch read-only.
    await waitFor(() => expect(store.isSavingLanguage).toBe(false));
    mocks.clear();
    rstest.restoreAllMocks();
    boot(false);
    await store.speak('en');
    store.setHardwareCapabilities(null);
});

describe('LanguageSwitch', () => {
    test('a device that does not offer its languages shows no switch', async () => {
        const listed = rstest.fn(() => ({ languages }));
        registerMocks(pb.LanguageService, { listLanguages: listed });
        boot(false);

        renderSwitch();

        expect(screen.queryByRole('combobox')).toBeNull();
        expect(listed).not.toHaveBeenCalled();
    });

    test('a language the device no longer lists selects nothing and says so, as the settings picker does', async () => {
        registerMocks(pb.LanguageService, { listLanguages: () => ({ languages }) });
        boot(true, null);

        renderSwitch();
        const closed = await screen.findByRole('combobox');
        expect(within(closed).getByText('Language no longer available')).toBeTruthy();
        fireEvent.click(closed);

        const options = screen.getAllByRole('option');
        expect(options.map(x => x.textContent)).toEqual(['English (UK)', '简体中文']);
        expect(options.filter(x => x.getAttribute('aria-selected') === 'true')).toHaveLength(0);
    });

    test('a pick made while the language is no longer available is saved and spoken', async () => {
        const setLanguage = rstest.fn(() => ({}));
        registerMocks(pb.LanguageService, {
            listLanguages: () => ({ languages }),
            getWebCatalog: () => ({ messages: { save: '保存' } }),
        });
        registerMocks(pb.services.InitialSetupService, { setLanguage });
        boot(true, null);

        renderSwitch();
        fireEvent.click(await screen.findByRole('combobox'));
        fireEvent.click(screen.getByText('简体中文'));

        await waitFor(() => expect(store.locale.tag).toBe('zh'));
        expect(setLanguage).toHaveBeenCalledWith(
            expect.objectContaining({ req: expect.objectContaining({ language: 'zh' }) }),
        );
    });

    test("while a pick saves, its switch is not disabled and the next step's switch opens no menu", async () => {
        const setLanguage = rstest.fn(({ conf }: { conf: (x: { delay: number }) => void }) => {
            conf({ delay: 50 });
            return {};
        });
        registerMocks(pb.LanguageService, {
            listLanguages: () => ({ languages }),
            getWebCatalog: () => ({ messages: { save: '保存' } }),
        });
        registerMocks(pb.services.InitialSetupService, { setLanguage });
        boot(true);

        renderSwitch();
        fireEvent.click(await screen.findByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));
        await waitFor(() => expect(store.isSavingLanguage).toBe(true));
        // A `disabled` field drops the focus of whoever just picked; a read-only one keeps it.
        expect(screen.getByRole<HTMLButtonElement>('combobox').disabled).toBe(false);

        cleanup();
        renderSwitch();
        fireEvent.click(await screen.findByText('English (UK)'));
        expect(screen.queryAllByRole('option')).toHaveLength(0);

        await waitFor(() => expect(store.isSavingLanguage).toBe(false));
    });

    test('a pick goes to the setup, which needs no session, and the page speaks it', async () => {
        const setLanguage = rstest.fn(() => ({}));
        registerMocks(pb.LanguageService, {
            listLanguages: () => ({ languages }),
            getWebCatalog: () => ({ messages: { save: '保存' } }),
        });
        registerMocks(pb.services.InitialSetupService, { setLanguage });
        boot(true);

        renderSwitch();
        fireEvent.click(await screen.findByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));

        await waitFor(() => expect(store.locale.tag).toBe('zh'));
        expect(setLanguage).toHaveBeenCalledWith(
            expect.objectContaining({ req: expect.objectContaining({ language: 'zh' }) }),
        );
    });

    test('a pick the page cannot follow shows what the device took on the next step too, so it can be set back', async () => {
        const setLanguage = rstest.fn(() => ({}));
        registerMocks(pb.LanguageService, {
            listLanguages: () => ({ languages }),
            getWebCatalog: () => {
                throw new ConnectError('no catalog', Code.Unavailable);
            },
        });
        registerMocks(pb.services.InitialSetupService, { setLanguage });
        rstest.spyOn(console, 'warn').mockImplementation(() => {});
        boot(true);

        const onErrors = renderSwitch();
        fireEvent.click(await screen.findByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));
        await waitFor(() => expect(onErrors).toHaveBeenLastCalledWith([expect.stringContaining('reload')]));
        expect(store.locale.tag).toBe('en-GB');

        // Each setup step mounts a switch of its own.
        cleanup();
        renderSwitch();
        fireEvent.click(await screen.findByText('简体中文'));
        fireEvent.click(screen.getByText('English (UK)'));

        await waitFor(() =>
            expect(setLanguage).toHaveBeenLastCalledWith(
                expect.objectContaining({ req: expect.objectContaining({ language: 'en-GB' }) }),
            ),
        );
    });

    test.each([
        ['a lost connection', new ConnectError('Failed to fetch', Code.Unknown)],
        ['a refusal', new ConnectError('refused by the device', Code.FailedPrecondition)],
    ])('%s is worded by the page and its cause logged', async (_case, failure) => {
        // A catalog that loads, so only the failed save keeps the page in its language.
        registerMocks(pb.LanguageService, {
            listLanguages: () => ({ languages }),
            getWebCatalog: () => ({ messages: { save: '保存' } }),
        });
        registerMocks(pb.services.InitialSetupService, {
            setLanguage: () => {
                throw failure;
            },
        });
        const warn = rstest.spyOn(console, 'warn').mockImplementation(() => {});
        boot(true);

        const onErrors = renderSwitch();
        fireEvent.click(await screen.findByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));

        await waitFor(() => expect(onErrors).toHaveBeenLastCalledWith(['Failed to change the language']));
        expect(warn).toHaveBeenCalledWith('Language not saved', failure);
        // The page switches only to a language the device took.
        expect(store.locale.tag).toBe('en-GB');
        expect(store.unspokenLanguage).toBeNull();
    });
});
