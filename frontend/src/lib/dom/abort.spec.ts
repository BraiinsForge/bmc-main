import { vi, describe, test, expect, beforeEach } from 'vitest';

import { Component, createElement } from 'react';
import { cleanup, render } from '@testing-library/react/pure';

import { abort, Aborter } from './abort';
import { ConnectError, Code } from '@connectrpc/connect';

describe('lib/abort', () => {
    describe('Aborter', () => {
        test('Has AbortController interface methods', () => {
            const onAbortOne = vi.fn();
            const $ = new Aborter(onAbortOne);

            expect($.signal).toBeInstanceOf(AbortSignal);
            expect($.abort).toBeInstanceOf(Function);
            expect(onAbortOne).toHaveBeenCalledTimes(0);

            $.abort();
            expect($.signal.aborted).toBe(true);
            expect(onAbortOne).toHaveBeenCalledTimes(1);

            const oldSignal = $.signal;
            const onAbortTwo = vi.fn();
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
            const onAbort = vi.fn();
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
                const onAbort = vi.fn();
                class Foo extends Component {
                    // @ts-expect-error: Called on unmount
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
            const onAbort = vi.fn();
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
