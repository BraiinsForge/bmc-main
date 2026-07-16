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

import { describe, test, expect, beforeEach, rstest } from '@rstest/core';

import { Component, createElement } from 'react';
import { cleanup, render } from '@testing-library/react/pure';

import { abort, Aborter } from './abort';
import { ConnectError, Code } from '@connectrpc/connect';

describe('lib/abort', () => {
    describe('Aborter', () => {
        test('Has AbortController interface methods', () => {
            const onAbortOne = rstest.fn();
            const $ = new Aborter(onAbortOne);

            expect($.signal).toBeInstanceOf(AbortSignal);
            expect($.abort).toBeInstanceOf(Function);
            expect(onAbortOne).toHaveBeenCalledTimes(0);

            $.abort();
            expect($.signal.aborted).toBe(true);
            expect(onAbortOne).toHaveBeenCalledTimes(1);

            const oldSignal = $.signal;
            const onAbortTwo = rstest.fn();
            $.replace(onAbortTwo as any);
            expect($.signal).not.toBe(oldSignal);
            expect($.signal.aborted).toBe(false);
            expect(onAbortTwo).toHaveBeenCalledTimes(0);
        });

        test('replace', () => {
            const aborter = new Aborter();
            const initialSignal = aborter.signal;
            aborter.replace();
            const newSignal = aborter.signal;

            expect(initialSignal).not.toBe(newSignal);
            expect(initialSignal.aborted).toBe(true);
            expect(newSignal.aborted).toBe(false);
        });

        describe('attach', () => {
            test('from wrapper to children', () => {
                const wrapper = new Aborter();
                const other1 = new Aborter();
                const other2 = new Aborter();

                wrapper.attach(other1);
                wrapper.attach(other2.signal);

                expect(wrapper).toHaveLength(3);
                expect(other1.signal.aborted).toBe(false);
                expect(other2.signal.aborted).toBe(false);

                wrapper.abort();

                // Wrapper & child attached via Aborter are both aborted
                expect(wrapper.signal.aborted).toBe(true);
                expect(other1.signal.aborted).toBe(true);

                // Cannot abort child attached via signal
                expect(other2.signal.aborted).toBe(false);
            });

            test('from child to wrapper', () => {
                const wrapper = new Aborter();
                const other1 = new Aborter();
                const other2 = new Aborter();

                wrapper.attach(other1);
                wrapper.attach(other2.signal);

                expect(wrapper).toHaveLength(3);
                expect(other1.signal.aborted).toBe(false);
                expect(other2.signal.aborted).toBe(false);

                // Abort through child attached via signal
                other2.abort();

                // Everything is aborted
                expect(wrapper.signal.aborted).toBe(true);
                expect(other1.signal.aborted).toBe(true);
                expect(other2.signal.aborted).toBe(true);
            });
        });
    });

    describe('abort (AbortCtrlHost)', () => {
        test('get', () => {
            const onAbort = rstest.fn();
            const $ = abort.get(onAbort);
            expect($).toBeInstanceOf(Aborter);

            $.abort();
            expect(onAbort).toHaveBeenCalledTimes(1);
        });

        test('is', () => {
            const plain = new Error();
            expect(abort.is(plain)).toBe(false);

            const named = new DOMException('undefined', 'AbortError');
            expect(abort.is(named)).toBe(true);

            const code = new ConnectError('xxx', Code.Canceled);
            expect(abort.is(code)).toBe(true);
        });

        describe('all', () => {
            beforeEach(cleanup);

            test('React component', () => {
                const onAbort = rstest.fn();
                class Foo extends Component {
                    // @ts-expect-error: Called on unmount
                    // biome-ignore lint/correctness/noUnusedPrivateClassMembers: Called in unmount
                    private abort = abort.get(onAbort);
                    componentWillUnmount = () => abort.all(this);
                    render = () => null;
                }

                const { unmount } = render(createElement(Foo));
                unmount();

                expect(onAbort).toHaveBeenCalledTimes(1);
            });
        });

        test('combine', () => {
            const onAbort = rstest.fn();
            const one = abort.get(onAbort);
            const two = abort.get(onAbort);
            const three = abort.get(onAbort);

            const $ = abort.combine(one, two, three);
            expect($.signal.aborted).toBe(false);

            $.abort();
            expect($.signal.aborted).toBe(true);
            expect(onAbort).toHaveBeenCalledTimes(3);

            expect(one.signal.aborted).toBe(true);
            expect(two.signal.aborted).toBe(true);
            expect(three.signal.aborted).toBe(true);
        });
    });
});
