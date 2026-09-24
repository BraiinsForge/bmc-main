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

import { describe, test, expect } from '@rstest/core';

import { deferred, serial } from './async';

describe('lib/async', () => {
    describe('serial', () => {
        test('a later task waits for an earlier one still in flight', async () => {
            const run = serial();
            const first = deferred<void>();
            const order: string[] = [];

            const a = run(async () => {
                await first;
                order.push('a');
            });
            const b = run(async () => {
                order.push('b');
            });

            first.resolve();
            await Promise.all([a, b]);

            expect(order).toEqual(['a', 'b']);
        });

        test('a failed task still lets the next one run', async () => {
            const run = serial();

            const failed = run(() => Promise.reject(new Error('boom')));
            const next = run(async () => 'ran');

            await expect(failed).rejects.toThrow('boom');
            await expect(next).resolves.toBe('ran');
        });
    });
});
