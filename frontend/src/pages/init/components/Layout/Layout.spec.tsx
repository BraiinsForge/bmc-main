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
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';
import { Code, ConnectError } from '@connectrpc/connect';

import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import type { ServiceMocks } from '@/lib/proto';
import { store } from '@/store';
import { deckSystemWire } from '@/pages/workspace/Display/capabilities.fixture';

import { Layout } from './Layout';

// The real routes import every page, and the store needs only `navigate`.
rstest.mock('@/routes', () => ({ default: { navigate: rstest.fn() } }));

type AnyService = Parameters<typeof mocks.service>[0];
function registerMocks<S extends AnyService>(service: S, methods: Partial<ServiceMocks<S>>): void {
    mocks.service(service, methods as ServiceMocks<S>);
}

const languages = [
    pb.create(pb.LanguageInfoSchema, { tag: 'en-GB', nativeName: 'English (UK)' }),
    pb.create(pb.LanguageInfoSchema, { tag: 'zh', nativeName: '简体中文' }),
];

function bootInSetup(): void {
    Object.assign(window, { SYSTEM: { ...deckSystemWire(), i18n: true, language: { tag: 'en-GB' } } });
    store.boot();
    Reflect.deleteProperty(window, 'SYSTEM');
}

function renderLayout(): void {
    render(
        <IntlProvider locale="en">
            <Layout header="Setup" children="the step" />
        </IntlProvider>,
    );
}

afterEach(async () => {
    cleanup();
    await waitFor(() => expect(store.isSavingLanguage).toBe(false));
    mocks.clear();
    rstest.restoreAllMocks();
    await store.speak('en');
});

describe('<Layout /> of the setup pages', () => {
    test('offers the device language in its header', async () => {
        registerMocks(pb.LanguageService, { listLanguages: () => ({ languages }) });
        bootInSetup();

        renderLayout();

        expect(await screen.findByText('English (UK)')).toBeTruthy();
    });

    test('shows a pick the device refused above the step', async () => {
        registerMocks(pb.LanguageService, { listLanguages: () => ({ languages }) });
        registerMocks(pb.services.InitialSetupService, {
            setLanguage: () => {
                throw new ConnectError('refused by the device', Code.FailedPrecondition);
            },
        });
        rstest.spyOn(console, 'warn').mockImplementation(() => {});
        bootInSetup();

        renderLayout();
        fireEvent.click(await screen.findByText('English (UK)'));
        fireEvent.click(screen.getByText('简体中文'));

        expect(await screen.findByText('Failed to change the language')).toBeTruthy();
    });
});
