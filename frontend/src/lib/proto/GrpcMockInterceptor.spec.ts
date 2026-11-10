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

import { describe, expect, test } from '@rstest/core';
import type { ServiceMocks } from '@/lib/proto';
import * as pb from '@/proto';
import { getClient, mocks } from '@/proto/transport';

const SETTLE_WINDOW_MS = 500;

type AccountMocks = ServiceMocks<typeof pb.services.AccountManagementService>;

describe('a unary call whose signal is aborted before it starts', () => {
    // Connect aborts the request signal before any interceptor runs,
    // so an `abort` listener added there never fires.
    const callAborted = (getAllAccounts: AccountMocks['getAllAccounts']): Promise<string> => {
        mocks.service(pb.services.AccountManagementService, { getAllAccounts } as AccountMocks);
        const aborted = new AbortController();
        aborted.abort();
        const call = getClient(pb.services.AccountManagementService).getAllAccounts({}, { signal: aborted.signal });
        return Promise.race([
            call.then(
                () => 'resolved',
                () => 'rejected',
            ),
            new Promise<string>(resolve => setTimeout(() => resolve('hung'), SETTLE_WINDOW_MS)),
        ]);
    };

    test('rejects instead of hanging the caller', async () => {
        expect(await callAborted(() => ({ accounts: [] }))).toBe('rejected');
    });

    test('never reaches the mock, as fetch never sends the request', async () => {
        let mockCalls = 0;
        await callAborted(() => {
            mockCalls += 1;
            return { accounts: [] };
        });
        expect(mockCalls).toBe(0);
    });
});
