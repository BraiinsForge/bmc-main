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

import { beforeEach, describe, expect, test } from '@rstest/core';
import { cleanup, render, fireEvent } from '@testing-library/react/pure';
import { IntlContext } from 'react-intl';
import * as pb from '@/proto';
import type { FormifiedValue } from '../../fn';
import { paramDef } from '../../fn/test-helpers';
import { FormWidgetManifest } from './FormWidgetManifest';

if (typeof ResizeObserver === 'undefined') {
    global.ResizeObserver = class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
    };
}

beforeEach(cleanup);

const fakeIntl = {
    formatMessage: ({ defaultMessage }: { defaultMessage?: string; id?: string }) => defaultMessage ?? '',
    locale: 'en',
    defaultLocale: 'en',
    timeZone: undefined,
    formats: {},
    defaultFormats: {},
    messages: {},
    onError: () => {},
} as unknown as ReturnType<typeof import('react-intl').useIntl>;

const wrap = (ui: ReactElement) => <IntlContext.Provider value={fakeIntl}>{ui}</IntlContext.Provider>;

describe('FormWidgetManifest', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'w',
        name: 'W',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [paramDef('paramInteger', 'count')],
    });

    test('renders per-field error from errors.fields[key]', () => {
        const { getByText } = render(
            wrap(
                <FormWidgetManifest
                    isOpen
                    onSave={() => {}}
                    onCancel={() => {}}
                    manifest={manifest}
                    params={{ count: 'abc' }}
                    errors={{ global: [], fields: { count: ['Not an integer'] } }}
                    onParamChange={() => {}}
                    timezones={[]}
                />,
            ),
        );
        expect(getByText('Not an integer')).toBeTruthy();
    });

    test('onParamChange propagates numeric input as a string', () => {
        let captured: [string, FormifiedValue] | null = null;
        render(
            wrap(
                <FormWidgetManifest
                    isOpen
                    onSave={() => {}}
                    onCancel={() => {}}
                    manifest={manifest}
                    params={{ count: '' }}
                    errors={null}
                    onParamChange={(k, v) => {
                        captured = [k, v];
                    }}
                    timezones={[]}
                />,
            ),
        );
        const input = document.body.querySelector<HTMLInputElement>('input[type="number"]');
        expect(input).toBeTruthy();
        if (!input) throw new Error('numeric input not found');
        fireEvent.change(input, { target: { value: '42' } });
        expect(captured).toEqual(['count', '42']);
    });
});
