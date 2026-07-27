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
import { IntlProvider } from 'react-intl';
import * as pb from '@/proto';
import { ParamField } from './ParamField';

beforeEach(cleanup);

const stringField = (format: pb.StringFormat) =>
    pb.create(pb.ManifestParamDefinitionSchema, {
        key: 'token',
        name: 'API Token',
        kind: { case: 'paramString', value: pb.create(pb.ParamStringSchema, { format }) },
    });

const renderField = (format: pb.StringFormat, value = 'sk-secret') =>
    render(
        <IntlProvider locale="en">
            <ParamField id="f" definition={stringField(format)} value={value} onChange={() => {}} timezones={[]} />
        </IntlProvider>,
    );

const input = () => document.body.querySelector<HTMLInputElement>('#f');

describe('ParamField password format', () => {
    test('hides the value until revealed', () => {
        renderField(pb.StringFormat.PASSWORD);
        expect(input()?.type).toBe('password');
    });

    test('reveals the value on toggle, and hides it again', () => {
        const { getByRole } = renderField(pb.StringFormat.PASSWORD);
        const toggle = getByRole('button');

        fireEvent.click(toggle);
        expect(input()?.type).toBe('text');

        fireEvent.click(toggle);
        expect(input()?.type).toBe('password');
    });

    test('a plain string format has no reveal toggle', () => {
        const { queryByRole } = renderField(pb.StringFormat.UNSPECIFIED);

        expect(input()?.type).toBe('text');
        expect(queryByRole('button')).toBeNull();
    });
});
